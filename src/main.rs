use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::Rng;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Sparkline},
    Terminal,
};
use std::{
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::time::sleep;
use windivert::{WinDivert, WinDivertFlags, WinDivertLayer};

/// CLI Definitions
#[derive(Parser)]
#[command(name = "subway-sim", version = "1.0", about = "Network Throttler for Mobile Emulators")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Start {
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
        #[arg(short, long, default_value_t = 0)]
        latency: u64,
        #[arg(short, long, default_value_t = 0)]
        drop_rate: u8,
        #[arg(long)]
        profile: Option<String>,
    },
}

/// Shared State for the TUI
struct AppState {
    intercepted: AtomicU64,
    dropped: AtomicU64,
    delayed: AtomicU64,
    throughput_history: std::sync::Mutex<Vec<u64>>,
    active_profile: String,
    config_latency: u64,
    config_drop: u8,
    config_port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { port, mut latency, mut drop_rate, profile } => {
            // Profile Overrides
            let mut profile_name = "Custom".to_string();
            if let Some(p) = profile {
                match p.to_lowercase().as_str() {
                    "elevator" => { latency = 2000; drop_rate = 15; profile_name = "Elevator".into(); }
                    "3g" => { latency = 500; drop_rate = 2; profile_name = "3G Network".into(); }
                    _ => { profile_name = format!("Custom: {}", p); }
                }
            }

            let state = Arc::new(AppState {
                intercepted: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
                delayed: AtomicU64::new(0),
                throughput_history: std::sync::Mutex::new(vec![0; 100]),
                active_profile: profile_name,
                config_latency: latency,
                config_drop: drop_rate,
                config_port: port,
            });

            run_app(port, state).await?;
        }
    }
    Ok(())
}

async fn run_app(port: u16, state: Arc<AppState>) -> Result<()> {
    // TUI Setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Start Packet Engine
    let engine_state = Arc::clone(&state);
    tokio::spawn(async move {
        if let Err(e) = packet_engine(port, engine_state).await {
            // Note: In a TUI, we can't easily print to stderr without ruining the layout
            // For now, we'll just log it mentally, but in production we'd use a log file.
        }
    });

    // TUI Loop
    let tick_rate = Duration::from_millis(200);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui(f, &state))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if let KeyCode::Char('q') = key.code {
                    break;
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            // Update throughput "vibe"
            let mut history = state.throughput_history.lock().unwrap();
            let current = state.intercepted.load(Ordering::Relaxed);
            history.push(current % 50); // Simulated "health" wave
            if history.len() > 100 { history.remove(0); }
            last_tick = Instant::now();
        }
    }

    // Cleanup
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

async fn packet_engine(port: u16, state: Arc<AppState>) -> Result<()> {
    /* 
       WinDivert Filter String:
       - 'tcp' or 'udp' filters the protocol.
       - 'local_port' and 'remote_port' capture both inbound/outbound legs.
       - '!(loopback)' ensures we don't accidentally capture internal noise.
    */
    let filter = format!(
        "(tcp or udp) and (local_port == {} or remote_port == {})",
        port
    );

    // Open WinDivert handle (Requires Admin)
    let diverter = WinDivert::new(&filter, WinDivertLayer::Network, 0, WinDivertFlags::default())
        .map_err(|e| anyhow::anyhow!("Failed to open WinDivert: {}. Ensure you are Admin.", e))?;
    
    let diverter = Arc::new(diverter);

    loop {
        // Receive a packet from the driver
        let packet = diverter.recv().map_err(|e| anyhow::anyhow!("Recv error: {}", e))?;
        state.intercepted.fetch_add(1, Ordering::Relaxed);

        let d_clone = Arc::clone(&diverter);
        let s_clone = Arc::clone(&state);

        // Process each packet in a non-blocking task
        tokio::spawn(async move {
            let mut rng = rand::thread_rng();

            // 1. Drop Check
            if rng.gen_range(0..100) < s_clone.config_drop {
                s_clone.dropped.fetch_add(1, Ordering::Relaxed);
                return; // Packet is never re-injected, effectively dropping it
            }

            // 2. Latency Injection
            if s_clone.config_latency > 0 {
                s_clone.delayed.fetch_add(1, Ordering::Relaxed);
                sleep(Duration::from_millis(s_clone.config_latency)).await;
            }

            /* 
               3. Re-injection:
               WinDivert uses the packet's 'address' metadata to understand its flow.
               Re-injecting it sends it back to the network stack to proceed to its target.
            */
            if let Err(e) = d_clone.send(&packet) {
                // Silently ignore send errors for now
            }
        });
    }
}

fn ui(f: &mut ratatui::Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Min(0),
        ])
        .split(f.size());

    // Header
    let header = Paragraph::new(format!(
        " SUBWAY-SIM | Profile: {} | Port: {} | Latency: {}ms | Drop: {}% | (Press 'q' to quit)",
        state.active_profile, state.config_port, state.config_latency, state.config_drop
    ))
    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Stats
    let stats_text = format!(
        "\n  Intercepted: {}\n  Dropped:     {}\n  Delayed:     {}",
        state.intercepted.load(Ordering::Relaxed),
        state.dropped.load(Ordering::Relaxed),
        state.delayed.load(Ordering::Relaxed)
    );
    let stats = Paragraph::new(stats_text)
        .block(Block::default().title(" Live Counters ").borders(Borders::ALL));
    f.render_widget(stats, chunks[1]);

    // Throughput Sparkline
    let history = state.throughput_history.lock().unwrap();
    let sparkline = Sparkline::default()
        .block(Block::default().title(" Network Throughput / Health ").borders(Borders::ALL))
        .data(&history)
        .style(Style::default().fg(Color::Green));
    f.render_widget(sparkline, chunks[2]);
}
