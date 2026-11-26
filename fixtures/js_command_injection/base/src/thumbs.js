const child_process = require('child_process');

function makeThumb(file) {
  return child_process.execFileSync('convert', [file, '-resize', '64x64', file + '.thumb.png']);
}

module.exports = { makeThumb };
