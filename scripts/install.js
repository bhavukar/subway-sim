const fs = require('fs');
const path = require('path');
const axios = require('axios');
const os = require('os');

const binaryName = os.platform() === 'win32' ? 'subway-sim-windows.exe' : 'subway-sim-macos';
const targetName = os.platform() === 'win32' ? 'subway-sim.exe' : 'subway-sim';
const url = `https://github.com/YOUR_USERNAME/subway-sim/releases/latest/download/${binaryName}`;

async function download() {
    const binDir = path.join(__dirname, '..', 'bin');
    if (!fs.existsSync(binDir)) fs.mkdirSync(binDir);
    const dest = path.join(binDir, targetName);

    console.log(`🚀 Downloading ${binaryName} for ${os.platform()}...`);
    const response = await axios({ url, method: 'GET', responseType: 'stream' });
    const writer = fs.createWriteStream(dest);

    response.data.pipe(writer);

    return new Promise((resolve, reject) => {
        writer.on('finish', () => {
            fs.chmodSync(dest, 0o755); // Make it executable
            console.log('✅ Done! Run subway-sim start');
            resolve();
        });
        writer.on('error', reject);
    });
}

download().catch(err => {
    console.error('❌ Download failed:', err.message);
    process.exit(1);
});
