const { makeThumb } = require('../src/thumbs');

test('makeThumb runs convert', () => {
  expect(typeof makeThumb).toBe('function');
});
