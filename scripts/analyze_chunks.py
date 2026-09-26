import urllib.request
import re
from concurrent.futures import ThreadPoolExecutor

urls = [
    "https://muse.ai/_next/static/chunks/0-h89d6un8vvz.js",
    "https://muse.ai/_next/static/chunks/00gd9l0y_bxvf.js",
    "https://muse.ai/_next/static/chunks/09lz2_kjqq7a6.js",
    "https://muse.ai/_next/static/chunks/0a-m0md7crzjm.js",
    "https://muse.ai/_next/static/chunks/0ax7ekf71s9_g.js",
    "https://muse.ai/_next/static/chunks/0bkar6mudhyd9.js",
    "https://muse.ai/_next/static/chunks/0er6uiayn9yba.js",
    "https://muse.ai/_next/static/chunks/0hetcbnyn0_sw.js",
    "https://muse.ai/_next/static/chunks/0iu6f5d3vbaha.js",
    "https://muse.ai/_next/static/chunks/0lbdhfi5jlzlk.js",
    "https://muse.ai/_next/static/chunks/0tc7lumc00zhe.js",
    "https://muse.ai/_next/static/chunks/0uxwb5cd6d58d.js",
    "https://muse.ai/_next/static/chunks/0v7t7lewr_m4-.js",
    "https://muse.ai/_next/static/chunks/0vqyeud950jg7.js",
    "https://muse.ai/_next/static/chunks/1-jdp15uxkaj9.js",
    "https://muse.ai/_next/static/chunks/1-upu88beu3ky.js",
    "https://muse.ai/_next/static/chunks/12hn_7io2gked.js",
    "https://muse.ai/_next/static/chunks/12n8t0zcmwh9o.js",
    "https://muse.ai/_next/static/chunks/13tcz3zdpkm82.js",
    "https://muse.ai/_next/static/chunks/15nhma8kgq6fp.js",
    "https://muse.ai/_next/static/chunks/15tcg-f4bm08k.js",
    "https://muse.ai/_next/static/chunks/17_11pi6_nzto.js",
    "https://muse.ai/_next/static/chunks/18sj5t_gn8_oc.js",
    "https://muse.ai/_next/static/chunks/190c3r15qietc.js",
    "https://muse.ai/_next/static/chunks/195pl7mjmocnr.js",
    "https://muse.ai/_next/static/chunks/1a2oyypht9flz.js",
    "https://muse.ai/_next/static/chunks/1a678s_var-dl.js",
    "https://muse.ai/_next/static/chunks/1bvhw100v8pqk.js",
    "https://muse.ai/_next/static/chunks/1ch5e_tf7r1of.js",
    "https://muse.ai/_next/static/chunks/1e09b9_-d4qgw.js",
    "https://muse.ai/_next/static/chunks/1exjrrxinjd78.js",
    "https://muse.ai/_next/static/chunks/1gebx__dvjhfb.js",
    "https://muse.ai/_next/static/chunks/1iimd4lfd301l.js",
    "https://muse.ai/_next/static/chunks/1ix9y_x0yegvd.js",
    "https://muse.ai/_next/static/chunks/1jsq9iy_yq51g.js",
    "https://muse.ai/_next/static/chunks/1kehvxhamale2.js",
    "https://muse.ai/_next/static/chunks/1kspl-8q0k75y.js",
    "https://muse.ai/_next/static/chunks/1mct11h6hc_8_.js",
    "https://muse.ai/_next/static/chunks/1nyi1vd99oy3b.js",
    "https://muse.ai/_next/static/chunks/1x576yvq2qk2m.js",
    "https://muse.ai/_next/static/chunks/1xpv1pt17ix_u.js",
    "https://muse.ai/_next/static/chunks/20co5fsa6kzju.js",
    "https://muse.ai/_next/static/chunks/24yq-insojbb7.js",
    "https://muse.ai/_next/static/chunks/25johm6o_j1dc.js",
    "https://muse.ai/_next/static/chunks/25l54261b_cvx.js",
    "https://muse.ai/_next/static/chunks/26i8c-axkhb5j.js",
    "https://muse.ai/_next/static/chunks/282_egzqv_1gs.js",
    "https://muse.ai/_next/static/chunks/285igq3ps2k29.js",
    "https://muse.ai/_next/static/chunks/2ap7neu2hh2l_.js",
    "https://muse.ai/_next/static/chunks/2fui--vcv5nje.js",
    "https://muse.ai/_next/static/chunks/2mg37u-hgdmlk.js",
    "https://muse.ai/_next/static/chunks/2o12axoangi7u.js",
    "https://muse.ai/_next/static/chunks/2plctbx-gihav.js",
    "https://muse.ai/_next/static/chunks/2tothwg2stt_0.js",
    "https://muse.ai/_next/static/chunks/2wmgxs5os-bol.js",
    "https://muse.ai/_next/static/chunks/2zxujh06i9lm7.js",
    "https://muse.ai/_next/static/chunks/31bmglu_mxblf.js",
    "https://muse.ai/_next/static/chunks/31lf6ocb31c6q.js",
    "https://muse.ai/_next/static/chunks/35gxeufero3zr.js",
    "https://muse.ai/_next/static/chunks/3_o8j1s531ru8.js",
    "https://muse.ai/_next/static/chunks/3bntu_nr6er55.js",
    "https://muse.ai/_next/static/chunks/3c5go35lzaopj.js",
    "https://muse.ai/_next/static/chunks/3hgn1lsqs9p69.js",
    "https://muse.ai/_next/static/chunks/3kbu7_3ma299c.js",
    "https://muse.ai/_next/static/chunks/3kg8syvtofv4o.js",
    "https://muse.ai/_next/static/chunks/3uqibdf8sq-tf.js",
    "https://muse.ai/_next/static/chunks/3ys45t9p3jr05.js",
    "https://muse.ai/_next/static/chunks/3z_95619zq2-l.js",
    "https://muse.ai/_next/static/chunks/452lto-hrezxp.js"
]

def fetch(url):
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as r:
            code = r.read().decode('utf-8', errors='ignore')
            matches = []
            if 'noise' in code.lower() or '25519' in code.lower() or 'chacha' in code.lower() or 'metaaivm' in code.lower() or 'hatch' in code.lower() or 'vm_id' in code.lower() or 'v1/noise' in code.lower():
                matches.append("CRYPTO/WEBSOCKET KEYWORDS FOUND")
            if matches:
                return f"{url}: {', '.join(matches)}"
    except Exception as e:
        return f"{url}: ERROR {e}"

with ThreadPoolExecutor(max_workers=10) as ex:
    for res in ex.map(fetch, urls):
        if res:
            print(res)
