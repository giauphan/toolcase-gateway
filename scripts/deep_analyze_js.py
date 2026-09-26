import urllib.request
import re

found_urls = [
    "https://muse.ai/_next/static/chunks/0-h89d6un8vvz.js",
    "https://muse.ai/_next/static/chunks/00gd9l0y_bxvf.js",
    "https://muse.ai/_next/static/chunks/0a-m0md7crzjm.js",
    "https://muse.ai/_next/static/chunks/0ax7ekf71s9_g.js",
    "https://muse.ai/_next/static/chunks/0bkar6mudhyd9.js",
    "https://muse.ai/_next/static/chunks/0hetcbnyn0_sw.js",
    "https://muse.ai/_next/static/chunks/0lbdhfi5jlzlk.js",
    "https://muse.ai/_next/static/chunks/0uxwb5cd6d58d.js",
    "https://muse.ai/_next/static/chunks/1-upu88beu3ky.js",
    "https://muse.ai/_next/static/chunks/12n8t0zcmwh9o.js",
    "https://muse.ai/_next/static/chunks/15tcg-f4bm08k.js",
    "https://muse.ai/_next/static/chunks/18sj5t_gn8_oc.js",
    "https://muse.ai/_next/static/chunks/1bvhw100v8pqk.js",
    "https://muse.ai/_next/static/chunks/1jsq9iy_yq51g.js",
    "https://muse.ai/_next/static/chunks/1kspl-8q0k75y.js",
    "https://muse.ai/_next/static/chunks/1xpv1pt17ix_u.js",
    "https://muse.ai/_next/static/chunks/2fui--vcv5nje.js",
    "https://muse.ai/_next/static/chunks/2mg37u-hgdmlk.js",
    "https://muse.ai/_next/static/chunks/31bmglu_mxblf.js",
    "https://muse.ai/_next/static/chunks/3c5go35lzaopj.js",
    "https://muse.ai/_next/static/chunks/3uqibdf8sq-tf.js",
    "https://muse.ai/_next/static/chunks/452lto-hrezxp.js"
]

patterns = [
    r'Noise_[A-Za-z0-9_]+',
    r'v1/noise',
    r'metaaivm',
    r'hatch/vm',
    r'WebSocket\(',
    r'class\s+[A-Za-z0-9_]*Noise[A-Za-z0-9_]*',
    r'function\s+[A-Za-z0-9_]*Noise[A-Za-z0-9_]*',
]

for url in found_urls:
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as r:
            code = r.read().decode('utf-8', errors='ignore')
            for p in patterns:
                matches = re.findall(p, code, re.IGNORECASE)
                if matches:
                    print(f"Match in {url} for pattern {p}: {set(matches)}")
                    # Find context around first match
                    m = re.search(p, code, re.IGNORECASE)
                    if m:
                        start = max(0, m.start() - 100)
                        end = min(len(code), m.end() + 100)
                        print(f"Context: {code[start:end]!r}\n")
    except Exception as e:
        print(f"Error {url}: {e}")
