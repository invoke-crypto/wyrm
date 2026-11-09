// The hero. One SVG; the mood attribute on the root selects which parts move (see styles.css).
// `pile` is 0..1 and grows the hoard under the dragon with the reserve.
export function dragonSvg(mood, pile) {
  const coins = [];
  let seed = 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;
  const height = 26 + pile * 34;
  for (let i = 0; i < 140; i++) {
    const x = 30 + rnd() * 340;
    const peak = Math.max(0, 1 - Math.abs(x - 200) / 170);
    const y = 282 - rnd() * height * peak;
    const rx = 7 + rnd() * 4;
    coins.push(`<ellipse cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" rx="${rx.toFixed(1)}" ry="${(rx * 0.38).toFixed(1)}" class="coin${i % 5 === 0 ? ' coin-lit' : ''}"/>`);
  }
  const lift = (pile * 20).toFixed(1);
  return `
<svg class="dragon" data-mood="${mood}" viewBox="0 0 400 300" role="img" aria-labelledby="dragon-title">
  <title id="dragon-title">The Dragon, ${mood.toLowerCase()}</title>
  <defs>
    <linearGradient id="hide" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#7c62c4"/><stop offset=".55" stop-color="#4a3878"/><stop offset="1" stop-color="#2a1f45"/>
    </linearGradient>
    <linearGradient id="wingskin" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#9a5a7a"/><stop offset="1" stop-color="#4a2a48"/>
    </linearGradient>
    <linearGradient id="flame" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#fff1b8"/><stop offset=".35" stop-color="#ffb43d"/><stop offset="1" stop-color="#e8452a" stop-opacity="0"/>
    </linearGradient>
    <radialGradient id="glow" cx=".5" cy=".5" r=".5">
      <stop offset="0" stop-color="#ffcf5a" stop-opacity=".55"/><stop offset="1" stop-color="#ffcf5a" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <ellipse class="hoard-glow" cx="200" cy="270" rx="190" ry="44" fill="url(#glow)"/>
  <g class="pile">${coins.join('')}</g>
  <g class="beast" style="--lift:${lift}px">
    <g class="body">
      <path class="tail" d="M150 238 C 100 250, 52 246, 40 226 C 30 208, 52 196, 66 206 C 54 210, 52 224, 70 230 C 96 238, 120 232, 150 226 Z"/>
      <path class="hide" d="M120 232 C 104 200, 132 176, 170 180 C 204 184, 214 166, 230 146 L 256 156 C 246 186, 226 214, 196 226 C 172 236, 140 240, 120 232 Z"/>
      <path class="hide" d="M200 190 L 226 146 L 258 158 L 236 200 Z"/>
      <path class="facet" d="M132 214 L 160 196 L 190 204 L 214 178 M 160 196 L 172 226 M 190 204 L 196 226 M 214 178 L 236 160"/>
      <path class="spines" d="M150 180 l6 -12 l6 10 M168 180 l7 -13 l6 12 M186 182 l6 -13 l6 11 M206 172 l6 -13 l5 11 M222 156 l7 -12 l4 11"/>
    </g>
    <g class="wing">
      <path class="wing-skin" d="M176 184 L 150 108 L 196 78 L 214 112 L 240 104 L 222 170 Z"/>
      <path class="wing-ribs" d="M176 184 L 196 78 M 186 180 L 214 112 M 200 176 L 240 104"/>
    </g>
    <g class="head">
      <path class="horn" d="M258 124 L 244 90 L 270 116 Z M 276 116 L 278 84 L 290 114 Z"/>
      <path class="hide" d="M230 146 L 258 120 L 300 110 L 336 124 L 330 136 L 296 140 L 272 160 L 252 160 Z"/>
      <path class="facet" d="M258 120 L 272 140 L 300 110 M 272 140 L 296 140 M 272 140 L 252 160"/>
      <path class="jaw" d="M272 154 L 296 141 L 330 138 L 322 150 L 288 162 Z"/>
      <circle class="nostril" cx="328" cy="128" r="1.8"/>
      <path class="eye-shut" d="M284 124 q 7 5 14 0"/>
      <g class="eye-open"><ellipse cx="291" cy="123" rx="7" ry="4.6"/><rect class="pupil" x="290" y="119" width="2.2" height="8" rx="1"/></g>
      <g class="fire">
        <path d="M330 144 L 400 118 L 392 146 L 400 176 Z" fill="url(#flame)"/>
        <path class="fire-core" d="M330 144 L 384 134 L 380 146 L 386 160 Z"/>
      </g>
    </g>
    <g class="zzz" aria-hidden="true"><text x="322" y="96">z</text><text x="338" y="76">z</text><text x="356" y="54">z</text></g>
  </g>
</svg>`;
}

export function scaleSvg(status, label = '', listed = false) {
  return `<svg class="plate plate-${status}${listed ? ' forsale' : ''}" viewBox="0 0 60 68" aria-hidden="true" focusable="false">
  <path class="plate-body" d="M30 2 L 56 14 L 52 44 L 30 66 L 8 44 L 4 14 Z"/>
  <path class="plate-cut" d="M30 2 L 30 30 L 56 14 M 30 30 L 4 14 M 30 30 L 52 44 M 30 30 L 8 44"/>
  ${label ? `<text x="30" y="37" class="plate-num">${label}</text>` : ''}
</svg>`;
}
