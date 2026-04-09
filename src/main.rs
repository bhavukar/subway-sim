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
use windivert::prelude::*;

#[derive(Parser)]
#[command(name = "subway-sim", version = "1.0", about = "Mobile Network Simulator")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start simulating a network environment
    Start {
        /// The port to throttle (default: targets 8080, 3000, 8000, 5000)
        #[arg(short, long)]
        port: Option<u16>,
        
        /// Predefined environment: subway, elevator, mountain, 3g
        #[arg(short, long, default_value = "subway")]
        profile: String,

        /// Custom latency in ms (overrides profile)
        #[arg(short, long)]
        latency: Option<u64>,

        /// Custom drop rate % (overrides profile)
        #[arg(short, long)]
        drop: Option<u8>,
    },
}

struct AppState {
    intercepted: AtomicU64,
    dropped: AtomicU64,
    delayed: AtomicU64,
    throughput_history: std::sync::Mutex<Vec<u64>>,
    profile_name: String,
    latency: u64,
    drop_rate: u8,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { port, profile, latency, drop } => {
            let (p_name, p_lat, p_drop) = match profile.to_lowercase().as_str() {
                "subway" => ("Subway (Spotty)", 800, 10),
                "elevator" => ("Elevator (Dead Zone)", 2500, 20),
                "mountain" => ("Mountain (High Latency)", 4000, 5),
                "3g" => ("3G Network", 400, 2),
                _ => ("Custom", 0, 0),
            };

            let final_lat = latency.unwrap_or(p_lat);
            let final_drop = drop.unwrap_or(p_drop);

            let state = Arc::new(AppState {
                intercepted: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
                delayed: AtomicU64::new(0),
                throughput_history: std::sync::Mutex::new(vec![0; 100]),
                profile_name: p_name.to_string(),
                latency: final_lat,
                drop_rate: final_drop,
            });

            // If no port provided, we throttle the "Big 4" dev ports
            let ports = if let Some(p) = port { vec![p] } else { vec![8080, 3000, 8000, 5000] };
            
            run_app(ports, state).await?;
        }
    }
    Ok(())
}

async fn run_app(ports: Vec<u16>, state: Arc<AppState>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    for &port in &ports {
        let engine_state = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = packet_engine(port, engine_state).await;
        });
    }

    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|f| ui(f, &state, &ports))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if let KeyCode::Char('q') = key.code { break; }
            }
        }
        if last_tick.elapsed() >= Duration::from_millis(200) {
            let mut history = state.throughput_history.lock().unwrap();
            let current = state.intercepted.load(Ordering::Relaxed);
            history.push(current % 50);
            if history.len() > 100 { history.remove(0); }
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

async fn packet_engine(port: u16, state: Arc<AppState>) -> Result<()> {
    let filter = format!("(tcp or udp) and (local_port == {} or remote_port == {})", port, port);
    let diverter = WinDivert::<NetworkLayer>::network(&filter, 0, WinDivertFlags::default())?;
    let diverter = Arc::new(diverter);

    loop {
        let packet = diverter.recv(None)?;
        state.intercepted.fetch_add(1, Ordering::Relaxed);
        let d_clone = Arc::clone(&diverter);
        let s_clone = Arc::clone(&state);

        tokio::spawn(async move {
            let should_drop = {
                let mut rng = rand::thread_rng();
                rng.gen_range(0..100) < s_clone.drop_rate
            };

            if should_drop {
                s_clone.dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }

            if s_clone.latency > 0 {
                s_clone.delayed.fetch_add(1, Ordering::Relaxed);
                sleep(Duration::from_millis(s_clone.latency)).await;
            }
            let _ = d_clone.send(&packet);
        });
    }
}

fn ui(f: &mut ratatui::Frame, state: &AppState, ports: &[u16]) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(3), Constraint::Length(8), Constraint::Min(0)])
        .split(f.size());

    let header = Paragraph::new(format!(
        " 🚇 SUBWAY-SIM | Profile: {} | Latency: {}ms | Drop: {}% ",
        state.profile_name, state.latency, state.drop_rate
    ))
    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    .block(Block::default().borders(Borders::ALL).title(" Status "));
    f.render_widget(header, chunks[0]);

    let ports_str = ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
    let stats = Paragraph::new(format!(
        "\n  Monitoring Ports: [{}]\n\n  Packets Caught:  {}\n  Packets Dropped: {}\n  Packets Delayed: {}",
        ports_str,
        state.intercepted.load(Ordering::Relaxed),
        state.dropped.load(Ordering::Relaxed),
        state.delayed.load(Ordering::Relaxed)
    )).block(Block::default().title(" Live Simulation ").borders(Borders::ALL));
    f.render_widget(stats, chunks[1]);

    let history = state.throughput_history.lock().unwrap();
    let sparkline = Sparkline::default()
        .block(Block::default().title(" Network Stability ").borders(Borders::ALL))
        .data(&history)
        .style(Style::default().fg(Color::Green));
    f.render_widget(sparkline, chunks[2]);
}
