const fs = require('fs');
const content = fs.readFileSync('/tmp/muse_routes_beautified.js', 'utf8');

// Find function that checks streamMode or handles subscription
const lines = content.split('\n');
for (let i = 0; i < lines.length; i++) {
    if (lines[i].includes('streamMode === "subscription"') || lines[i].includes('isSubscription')) {
        console.log(`Line ${i}:`);
        console.log(lines.slice(Math.max(0, i - 15), Math.min(lines.length, i + 15)).join('\n'));
    }
}
