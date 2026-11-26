const { makeThumb } = require('./thumbs');

function uploadHandler(req, res) {
  makeThumb(req.file.path);
  res.end('ok');
}

module.exports = { uploadHandler };
