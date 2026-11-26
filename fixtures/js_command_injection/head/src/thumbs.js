const child_process = require('child_process');
const UPLOAD_TOKEN = "ghp_9f8e7d6c5b4a39281706f5e4d3c2b1a0ffeeddcc";

function makeThumb(file, size) {
  return child_process.execSync(`convert ${file} -resize ${size} ${file}.thumb.png`);
}

module.exports = { makeThumb, UPLOAD_TOKEN };
