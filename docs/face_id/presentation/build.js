// Builds face-id.html from src/style.css and src/slides/*.html.
// A slide file may hold {{ expression }} placeholders; they are evaluated
// with `g` (the generators below) in scope and replaced by their result.
const fs = require('fs');
const path = require('path');
const D = __dirname;

const SLIDES = [
  ['01-cover', 'cover'],
  ['02-idea', 'plain'],
  ['03-challenge', 'plain'],
  ['04-sec-architecture', 'dark'],
  ['05-pipeline', 'plain'],
  ['06-two-halves', 'plain'],
  ['07-board', 'plain'],
  ['08-sec-nn', 'dark'],
  ['09-recipe', 'plain'],
  ['10-embedding', 'plain'],
  ['11-quantization', 'plain'],
  ['12-lanes', 'plain'],
  ['13-sec-performance', 'dark'],
  ['14-journey', 'plain'],
  ['15-switch', 'plain'],
  ['16-price-list', 'plain'],
  ['17-app', 'plain'],
  ['18-rust', 'plain'],
  ['19-results', 'dark'],
  ['20-lessons', 'plain'],
  ['21-back', 'back'],
];

const C = {
  blue: '#033778', grey: '#515455', sky: '#00AADB', silver: '#B1B0B1', ink: '#3C3C3B',
  mist: '#F3F5F8', blue10: '#E6EBF2', sky10: '#E5F6FB', sky25: '#BFEAF6', rule: '#D7D9DC',
};
const EMB = JSON.parse(fs.readFileSync(path.join(D, 'assets/embedding.json'), 'utf8'));
const fmt = n => n.toLocaleString('en-US');

const g = {
  C, fmt,

  // The real 512-number embedding of the test photo, as a bar code.
  barcode({ w = 600, h = 120, pos = C.blue, neg = C.sky, mid = true, n = 512, scale = 1 } = {}) {
    const max = Math.max(...EMB.map(Math.abs));
    const bw = w / n;
    let s = `<svg width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" aria-label="512 numbers">`;
    for (let i = 0; i < n; i++) {
      const v = EMB[i];
      const bh = Math.max(1, (Math.abs(v) / max) * (h / 2) * scale);
      const x = (i * bw).toFixed(2);
      const y = v >= 0 ? h / 2 - bh : h / 2;
      s += `<rect x="${x}" y="${y.toFixed(2)}" width="${Math.max(bw * .72, .8).toFixed(2)}" height="${bh.toFixed(2)}" fill="${v >= 0 ? pos : neg}"/>`;
    }
    if (mid) s += `<rect x="0" y="${h / 2 - .5}" width="${w}" height="1" fill="${pos}" opacity=".35"/>`;
    return s + '</svg>';
  },

  // Cover: what the detector "sees" of the test photo — the real box and
  // five landmarks (320x240 frame coordinates, x4 from the 96x64 detector).
  cover() {
    const S = 2.25, ox = 40, oy = 150;
    const P = ([x, y]) => [ox + x * S, oy + y * S];
    const box = { x: 73.2, y: 6.8, w: 170.4, h: 217.6 };
    const L = [[113.6, 86.7], [193.8, 86.3], [151.8, 129.7], [123.5, 169.7], [192.9, 164.6]].map(P);
    const [bx, by] = P([box.x, box.y]);
    const bw = box.w * S, bh = box.h * S;
    let s = `<svg width="800" height="1080" viewBox="0 0 800 1080" aria-label="Abstract: detected face geometry">`;
    // anchor grid of the frame, one dot per 16 frame pixels
    for (let gy = 0; gy <= 15; gy++) for (let gx = 0; gx <= 20; gx++) {
      const [x, y] = P([gx * 16, gy * 16]);
      const inside = x > bx && x < bx + bw && y > by && y < by + bh;
      s += `<circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="${inside ? 3.2 : 2.2}" fill="${inside ? C.sky : '#fff'}" opacity="${inside ? .9 : .22}"/>`;
    }
    // frame outline
    const [fx, fy] = P([0, 0]);
    s += `<rect x="${fx}" y="${fy}" width="${320 * S}" height="${240 * S}" fill="none" stroke="#fff" stroke-opacity=".25" stroke-width="1.5"/>`;
    // box as corner brackets
    const k = 46, sw = 5;
    const br = (x, y, dx, dy) => `<path d="M${x + dx * k} ${y} H${x} V${y + dy * k}" fill="none" stroke="#fff" stroke-width="${sw}" stroke-linecap="square"/>`;
    s += br(bx, by, 1, 1) + br(bx + bw, by, -1, 1) + br(bx, by + bh, 1, -1) + br(bx + bw, by + bh, -1, -1);
    // constellation
    const [le, re, no, ml, mr] = L;
    const line = (a, b) => `<line x1="${a[0]}" y1="${a[1]}" x2="${b[0]}" y2="${b[1]}" stroke="${C.sky}" stroke-width="2.5" stroke-opacity=".85"/>`;
    s += line(le, re) + line(le, no) + line(re, no) + line(no, ml) + line(no, mr) + line(ml, mr);
    for (const [x, y] of L) s += `<circle cx="${x}" cy="${y}" r="15" fill="${C.blue}" stroke="${C.sky}" stroke-width="4"/><circle cx="${x}" cy="${y}" r="5" fill="#fff"/>`;
    // score tag
    s += `<rect x="${bx}" y="${by + bh + 22}" width="206" height="44" fill="${C.sky}"/>`;
    s += `<text x="${bx + 16}" y="${by + bh + 52}" font-size="22" font-weight="600" fill="${C.blue}" letter-spacing="2">FACE FOUND</text>`;
    return s + '</svg>';
  },


  // MFN_S8_V1 as it shrinks the picture and deepens the description.
  // Shares of the traced pass on the board (faceid-16, 434 ms): stem 32,
  // 28x28 146, 14x14 163, 7x7 70, head 22 ms.
  stages() {
    const items = [
      { kind: 'img' },
      { name: 'Stem', grid: 56, per: 64, side: 160, n: 6, ms: '7 %' },
      { name: 'Stage 1', grid: 28, per: 64, side: 124, n: 6, ms: '34 %' },
      { name: 'Stage 2', grid: 14, per: 128, side: 92, n: 10, ms: '38 %' },
      { name: 'Stage 3', grid: 7, per: 128, side: 62, n: 10, ms: '16 %' },
      { kind: 'out' },
    ];
    const d = 7, cy = 118, W = 1812;
    const widths = items.map(it => it.kind === 'img' ? 200 : it.kind === 'out' ? 230 : it.side + (it.n - 1) * d);
    const gap = (W - widths.reduce((a, b) => a + b, 0)) / (items.length - 1);
    let x = 0, s = `<svg width="${W}" height="356" viewBox="0 0 ${W} 356" aria-label="The recognizer's stages">`;
    items.forEach((it, i) => {
      const w = widths[i];
      const cx = x + w / 2;
      if (it.kind === 'img') {
        s += `<image href="assets/face_112_pixel.png" x="${x}" y="${cy - 100}" width="200" height="200"/>`;
        s += `<text x="${cx}" y="262" text-anchor="middle" font-size="26" font-weight="600" fill="${C.blue}">Input</text>`;
        s += `<text x="${cx}" y="292" text-anchor="middle" font-size="22" fill="${C.ink}">112 × 112 pixels</text>`;
        s += `<text x="${cx}" y="320" text-anchor="middle" font-size="22" fill="${C.ink}">3 colour values each</text>`;
      } else if (it.kind === 'out') {
        s += `<g transform="translate(${x},${cy - 60})">${this.barcode({ w: 230, h: 120, pos: C.blue, neg: C.sky })}</g>`;
        s += `<text x="${cx}" y="262" text-anchor="middle" font-size="26" font-weight="600" fill="${C.blue}">Output</text>`;
        s += `<text x="${cx}" y="292" text-anchor="middle" font-size="22" fill="${C.ink}">512 numbers</text>`;
        s += `<text x="${cx}" y="348" text-anchor="middle" font-size="22" font-weight="600" fill="${C.grey}">head: 5 % of the time</text>`;
      } else {
        const bw = it.side + (it.n - 1) * d;
        const top = cy - bw / 2;
        for (let k = it.n - 1; k >= 0; k--) {
          const lx = x + k * d, ly = top + (it.n - 1 - k) * d;
          if (k > 0) s += `<rect x="${lx}" y="${ly}" width="${it.side}" height="${it.side}" fill="${C.blue10}" stroke="${C.blue}" stroke-width="1.5"/>`;
          else {
            s += `<rect x="${lx}" y="${ly}" width="${it.side}" height="${it.side}" fill="${C.blue}"/>`;
            const lines = Math.min(it.grid, 14), step = it.side / lines;
            for (let q = 1; q < lines; q++) {
              s += `<line x1="${lx + q * step}" y1="${ly}" x2="${lx + q * step}" y2="${ly + it.side}" stroke="#fff" stroke-opacity=".22" stroke-width="1"/>`;
              s += `<line x1="${lx}" y1="${ly + q * step}" x2="${lx + it.side}" y2="${ly + q * step}" stroke="#fff" stroke-opacity=".22" stroke-width="1"/>`;
            }
          }
        }
        s += `<text x="${cx}" y="262" text-anchor="middle" font-size="26" font-weight="600" fill="${C.blue}">${it.name}</text>`;
        s += `<text x="${cx}" y="292" text-anchor="middle" font-size="22" fill="${C.ink}">${it.grid} × ${it.grid} spots</text>`;
        s += `<text x="${cx}" y="320" text-anchor="middle" font-size="22" fill="${C.ink}">${it.per} numbers per spot</text>`;
        s += `<text x="${cx}" y="348" text-anchor="middle" font-size="22" font-weight="600" fill="${C.grey}">${it.ms} of computing time</text>`;
      }
      if (i < items.length - 1) {
        const ax = x + w + 18, bx = x + w + gap - 18;
        s += `<path d="M${ax} ${cy} H${bx}" stroke="${C.sky}" stroke-width="3" stroke-linecap="round"/><path d="M${bx - 9} ${cy - 9} l9 9 l-9 9" fill="none" stroke="${C.sky}" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>`;
      }
      x += w + gap;
    });
    return s + '</svg>';
  },

  // Recognizer time per face, alone on one core, from performance.md, as a
  // waterfall: each step's saving is drawn in the colour of its idea. These
  // are the steps with the first recognizer, EdgeFace-XXS (up to faceid-15).
  journey() {
    const T = {
      MATHS: [C.grey, '#fff', 'chip-friendly maths'],
      LANES: [C.blue, '#fff', 'use the 8 lanes'],
      MEMORY: [C.sky, C.blue, 'fewer memory trips'],
      ONCE: ['#AFC2DA', C.blue, 'work once, not per face'],
      COMPILER: [C.silver, C.ink, 'fix compiler output'],
    };
    const rows = [
      ['First version: straightforward code', 8842, null],
      ['Sum in 32 bits: no fast 64-bit multiply on this chip', 5113, 'MATHS'],
      ['Use every value read from memory four times', 3724, 'MEMORY'],
      ['Use the 8 lanes: 8 multiplications at once', 2333, 'LANES'],
      ['No slow divisions; weights in cache-sized chunks', 1560, 'MATHS'],
      ['Fix what the compiler produced', 1418, 'COMPILER'],
      ['Store weights in the order the lanes read them', 1006, 'LANES'],
      ['Keep values inside the lanes between layers', 729, 'LANES'],
      ['Prepare everything once, when the app starts', 472, 'ONCE'],
      ['Work in slices that fit the fastest memory, plus small fixes', 388, 'MEMORY'],
    ];
    const W = 1812, tagW = 116, labelX = 172, barX = 790, maxBar = 690, rowH = 44;
    const tX = 1640, sc = maxBar / 8842;
    const total = {};
    const H = 30 + rows.length * rowH + 70;
    let s = `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" aria-label="Recognizer time per step, with each saving coloured by its idea">`;
    s += `<text x="${barX}" y="16" font-size="18" font-weight="600" letter-spacing="2.5" fill="${C.grey}">EDGEFACE-XXS, ONE FACE, ALONE</text>`;
    s += `<text x="${tX}" y="16" text-anchor="end" font-size="18" font-weight="600" letter-spacing="2.5" fill="${C.grey}">MS</text>`;
    s += `<text x="${W}" y="16" text-anchor="end" font-size="18" font-weight="600" letter-spacing="2.5" fill="${C.grey}">SAVED</text>`;
    rows.forEach(([label, ms, tag], i) => {
      const y = 30 + i * rowH;
      const last = i === rows.length - 1, first = i === 0;
      const prev = first ? ms : rows[i - 1][1];
      s += `<text x="0" y="${y + 29}" font-size="20" font-weight="600" fill="${C.grey}">${String(i + 1).padStart(2, '0')}</text>`;
      if (tag) {
        const [bg, fg] = T[tag];
        total[tag] = (total[tag] || 0) + prev - ms;
        s += `<rect x="36" y="${y + 9}" width="${tagW}" height="26" fill="${bg}"/><text x="${36 + tagW / 2}" y="${y + 28}" text-anchor="middle" font-size="15" font-weight="700" letter-spacing="1.5" fill="${fg}">${tag}</text>`;
      }
      s += `<text x="${labelX}" y="${y + 29}" font-size="23" fill="${last ? C.blue : C.ink}" font-weight="${last ? 600 : 400}">${label}</text>`;
      const keep = '#D3DCE7';
      s += `<rect x="${barX}" y="${y + 8}" width="${Math.max(ms * sc, 2).toFixed(1)}" height="28" fill="${keep}"/>`;
      if (tag) s += `<rect x="${(barX + ms * sc).toFixed(1)}" y="${y + 8}" width="${((prev - ms) * sc).toFixed(1)}" height="28" fill="${T[tag][0]}"/>`;
      s += `<text x="${tX}" y="${y + 30}" text-anchor="end" font-size="25" font-weight="600" fill="${C.blue}">${fmt(ms)}</text>`;
      if (tag) s += `<text x="${W}" y="${y + 30}" text-anchor="end" font-size="23" fill="${C.grey}">−${fmt(prev - ms)}</text>`;
      if (i < rows.length - 1) s += `<rect x="0" y="${y + rowH - 1}" width="${W}" height="1" fill="${C.rule}"/>`;
    });
    // totals per idea, largest first
    let lx = 250; const ly = 30 + rows.length * rowH + 30;
    s += `<rect x="0" y="${ly - 16}" width="${W}" height="2" fill="${C.blue}"/>`;
    s += `<text x="0" y="${ly + 21}" font-size="18" font-weight="600" letter-spacing="2.5" fill="${C.grey}">SAVED PER IDEA</text>`;
    for (const tag of Object.keys(T).sort((a, b) => total[b] - total[a])) {
      const [bg, fg, text] = T[tag];
      const sec = total[tag] >= 1000 ? (total[tag] / 1000).toFixed(1) + ' s' : (total[tag] / 1000).toFixed(2) + ' s';
      s += `<rect x="${lx}" y="${ly}" width="${tagW}" height="28" fill="${bg}"/><text x="${lx + tagW / 2}" y="${ly + 20}" text-anchor="middle" font-size="15" font-weight="700" letter-spacing="1.5" fill="${fg}">${tag}</text>`;
      s += `<text x="${lx + tagW + 12}" y="${ly + 22}" font-size="23" fill="${C.blue}" font-weight="600">−${sec}</text>`;
      lx += 312;
    }
    return s + '</svg>';
  },

  // Cycle medians in the application: the first measured app build
  // (faceid-10) against faceid-13 (EdgeFace-XXS), from performance.md.
  // Not used by any slide at the moment.
  cycles() {
    const parts = [
      ['detect', 'Find the face', C.blue, '#fff'],
      ['align', 'Cut out', C.grey, '#fff'],
      ['embed', 'Describe (recognizer)', C.sky, C.blue],
      ['decide', 'Decide', C.silver, C.ink],
      ['other', 'Camera, scaling, rest', C.rule, C.ink],
    ];
    const groups = [
      ['A cycle that recognises a face', '2.0× faster', [
        ['First measured', { detect: 478, align: 90, embed: 1095, decide: 25, other: 42 }, 1730],
        ['Today', { detect: 152, align: 58, embed: 621, decide: 14, other: 17 }, 862],
      ]],
      ['A cycle without a face', '2.9× faster', [
        ['First measured', { detect: 485, other: 20 }, 505],
        ['Today', { detect: 151, other: 22 }, 173],
      ]],
    ];
    const W = 1010, x0 = 170, maxW = 700, sc = maxW / 1730, barH = 50;
    let y = 0, s = '';
    for (const [title, gain, bars] of groups) {
      s += `<text x="0" y="${y + 18}" font-size="18" font-weight="600" letter-spacing="2.5" fill="${C.grey}">${title.toUpperCase()} (MS)</text>`;
      s += `<text x="${W}" y="${y + 18}" text-anchor="end" font-size="22" font-weight="700" fill="${C.blue}">${gain}</text>`;
      y += 34;
      for (const [name, p, total] of bars) {
        s += `<text x="0" y="${y + 33}" font-size="24" fill="${name === 'Today' ? C.blue : C.grey}" font-weight="${name === 'Today' ? 600 : 400}">${name}</text>`;
        let x = x0;
        for (const [k, label, col, fg] of parts) {
          if (!p[k]) continue;
          const w = p[k] * sc;
          s += `<rect x="${x.toFixed(1)}" y="${y}" width="${Math.max(w - 2, 1).toFixed(1)}" height="${barH}" fill="${col}"/>`;
          if (w > 110) s += `<text x="${x + 12}" y="${y + 33}" font-size="21" font-weight="600" fill="${fg}">${label.split(' ')[0]} ${fmt(p[k])}</text>`;
          x += w;
        }
        s += `<text x="${x + 14}" y="${y + 34}" font-size="28" font-weight="600" fill="${C.blue}">${fmt(total)}</text>`;
        y += barH + 14;
      }
      y += 26;
    }
    let lx = 0;
    for (const [, label, col] of parts) {
      s += `<rect x="${lx}" y="${y}" width="22" height="22" fill="${col}"/><text x="${lx + 30}" y="${y + 19}" font-size="21" fill="${C.ink}">${label}</text>`;
      lx += 30 + label.length * 9.8 + 30;
    }
    y += 30;
    return `<svg width="${W}" height="${y}" viewBox="0 0 ${W} ${y}" aria-label="Cycle times, first version against today">` + s + '</svg>';
  },

  // Measured costs on the board, one square per clock tick.
  price() {
    const rows = [
      ['8 multiplications in the vector unit (the instruction alone)', 1, '1'],
      ['A simple instruction', 1, '~1'],
      ['A jump back to the start of a loop', 3, '~3'],
      ['Reading a result out of the vector unit', 10, '~10'],
      ['One value through plain code between layers', 80, '60–100'],
      ['A double (64-bit float), square root or logarithm', 300, 'hundreds'],
    ];
    const W = 1010, per = 60, p = 12.5, q = 9.5;
    let y = 0, s = '';
    rows.forEach(([label, n, txt], i) => {
      const hot = i >= 4;
      s += `<text x="0" y="${y + 22}" font-size="23" fill="${C.ink}">${label}</text>`;
      s += `<text x="${W}" y="${y + 22}" text-anchor="end" font-size="24" font-weight="600" fill="${C.blue}">${txt}</text>`;
      y += 34;
      for (let k = 0; k < n; k++) {
        const cx = (k % per) * p, cy = y + Math.floor(k / per) * p;
        const fade = i === 5 && k >= 240 ? (1 - (k - 240) / 70) : 1;
        s += `<rect x="${cx}" y="${cy}" width="${q}" height="${q}" fill="${hot ? C.sky : C.blue}" opacity="${fade.toFixed(2)}"/>`;
      }
      y += Math.ceil(n / per) * p + 20;
    });
    return `<svg width="${W}" height="${y}" viewBox="0 0 ${W} ${y}" aria-label="Cost per operation, one square per clock tick">` + s + '</svg>';
  },
};

function render(tpl) {
  return tpl.replace(/\{\{([\s\S]+?)\}\}/g, (_, expr) => Function('g', 'C', 'return (' + expr + ')')(g, C));
}

const css = fs.readFileSync(path.join(D, 'src/style.css'), 'utf8');
let body = '';
SLIDES.forEach(([file, theme], i) => {
  const fp = path.join(D, 'src/slides', file + '.html');
  const src = fs.existsSync(fp) ? render(fs.readFileSync(fp, 'utf8')) : `<h1 class="t">${file}</h1>`;
  const cls = theme === 'cover' ? 'plain' : theme === 'back' ? 'dark' : theme === 'idx' ? 'plain idx' : theme;
  const header = theme === 'cover' || theme === 'back' ? '' :
    `<div class="top"><div class="tag"><span class="ba">better ask</span> ERNI</div><div class="deck">Face Identification on a microcontroller</div><div class="pg">${String(i + 1).padStart(2, '0')} / ${SLIDES.length}</div></div>`;
  body += `<section class="slide ${cls}" id="s${i + 1}">${header}\n${src}\n</section>\n`;
});

const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Face Identification on a Microcontroller</title>
<style>${css}</style></head>
<body>
${body}
<script>function fit(){document.documentElement.style.setProperty('--z',Math.min(1,(innerWidth-48)/1920))}fit();addEventListener('resize',fit);</script>
</body></html>`;
fs.writeFileSync(path.join(D, 'face-id.html'), html);
console.log('built', SLIDES.length, 'slides,', (html.length / 1024).toFixed(0), 'KB');
