const fs = require('fs');
const content = fs.readFileSync('/tmp/muse_routes_beautified.js', 'utf8');
const idx = content.indexOf('"/chat/stream"');
console.log(content.substring(Math.max(0, idx - 1000), idx + 1000));
