const { makeThumb } = require('./thumbs');

function uploadHandler(req, res) {
  makeThumb(req.file.path, req.query.size || '64x64');
  res.end('ok');
}

module.exports = { uploadHandler };
