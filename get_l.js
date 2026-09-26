const fs = require('fs');
const content = fs.readFileSync('/tmp/muse_protocol_beautified.js', 'utf8');
const lines = content.split('\n');
const start = Math.max(0, 1100);
const end = Math.min(lines.length, 1160);
console.log(lines.slice(start, end).join('\n'));
