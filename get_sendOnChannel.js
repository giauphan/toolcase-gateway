const fs = require('fs');
const content = fs.readFileSync('/tmp/auth_0uxwb5cd6d58d.js', 'utf8');

const idx = content.indexOf('sendOnChannel:(');
console.log(content.substring(Math.max(0, idx - 800), idx + 800));
