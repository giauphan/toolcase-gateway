const fs = require('fs');
const content = fs.readFileSync('/tmp/auth_1rdryc_7yby1g.js', 'utf8');

// The file might not have the subscription logic. Let's find files having "/chat/subscribe"
const execSync = require('child_process').execSync;
try {
    const out = execSync('grep -l "/chat/subscribe" /tmp/*.js').toString().split('\n');
    out.forEach(f => {
        if (f) {
            const buf = fs.readFileSync(f, 'utf8');
            const idx = buf.indexOf('"/chat/subscribe"');
            console.log(`File: ${f}`);
            console.log(buf.substring(Math.max(0, idx - 500), idx + 500));
            console.log('---');
        }
    });
} catch(e) {}
