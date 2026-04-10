#!/usr/bin/env node
const { spawn } = require('child_process');
const path = require('path');
const os = require('os');

const platform = os.platform();
const arch = os.arch();

let binaryName = '';

if (platform === 'win32') {
    binaryName = 'subway-sim-windows.exe';
} else if (platform === 'darwin') {
    binaryName = 'subway-sim-macos';
} else {
    console.error(`Error: platform ${platform} is not supported by subway-sim.`);
    process.exit(1);
}

const binaryPath = path.join(__dirname, binaryName);

// Forward all arguments to the binary
const args = process.argv.slice(2);
const child = spawn(binaryPath, args, {
    stdio: 'inherit',
    shell: false
});

child.on('error', (err) => {
    if (err.code === 'ENOENT') {
        console.error(`Error: Binary not found at ${binaryPath}`);
        console.error('Make sure you have installed the package correctly for your platform.');
    } else {
        console.error(`Error launching binary: ${err.message}`);
    }
    process.exit(1);
});

child.on('exit', (code) => {
    process.exit(code || 0);
});
