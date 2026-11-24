/* Shared behaviour of the G6 mockups: one state table with sample values, controls bound to it by
   data attributes, and the toolkit's appearance settings. Nothing here talks to a device. */
const G6 = (() => {
  'use strict';
  const PRESETS = /*PRESETS*/{};
  const FREQS = ['31', '62', '125', '250', '500', '1k', '2k', '4k', '8k', '16k'];
  const FILTERS = ['Fast Roll-off / Minimum Phase', 'Slow Roll-off / Minimum Phase', 'Fast Roll-off / Linear Phase', 'Slow Roll-off / Linear Phase'];
  const FX = [
    ['sur', 'Surround', 'sbx-surround', 'Widens the sound stage; virtual surround on headphones.'],
    ['cry', 'Crystalizer', 'sbx-crystalizer', 'Restores highs and lows lost in compressed audio.'],
    ['bass', 'Bass', 'sbx-bass', 'Boosts low frequencies. Crossover fixed at 80 Hz.'],
    ['smart', 'Smart Volume', 'sbx-smart-volume', 'Evens out loud and quiet passages.'],
    ['dlg', 'Dialog+', 'sbx-dialog-plus', 'Lifts voices in films and games.'],
  ];
  const MON = [['linein', 'Line In', 'line-in'], ['mic', 'External Mic', 'external-mic'], ['spdif', 'SPDIF In', 'spdif-in']];
  const REC = [...MON, ['wuh', 'What U Hear', 'what-u-hear']];

  // ---- sample state ---------------------------------------------------------------------------
  const S = {
    'ui.theme': 'System', 'ui.backdrop': 'Mica', 'ui.corners': 'Round', 'ui.wall': '1', 'ui.focus': true, 'ui.page': '', 'ui.dialog': false, 'ui.dlg': 'Appearance', 'ui.conn': true,
    firmware: '2.1.250903.1324', output: 'Speakers', vol: 34, mute: false, 'lay.spk': 'Stereo', 'lay.hp': 'Stereo', 'lay.cur': 'Stereo',
    direct: true, spdif: false, lineout: false, decoder: 'Normal', filter: FILTERS[0],
    sbx: false, scout: false,
    'sbx.sur': true, 'sbx.sur.v': 67, 'sbx.cry': true, 'sbx.cry.v': 50, 'sbx.bass': false, 'sbx.bass.v': 50,
    'sbx.smart': false, 'sbx.smart.v': 70, 'sbx.smart.mode': 'Normal', 'sbx.dlg': false, 'sbx.dlg.v': 50,
    eq: true, 'eq.preset': 'Rock', 'eq.pre': 0,
    'mic.boost': '20', 'mic.vol': 70, 'mic.mute': false, 'mon.vol': 30, 'mon.mute': true,
    vc: true, 'vc.nr': 60, 'vc.aec': false, 'vc.smart': false, 'vc.eq': false, 'vc.eqp': 'Preset 1',
    'mix.play.mute': false,
    'light': true, 'light.color': '#ff2a2a', 'light.ring': true, 'light.fx': 'Off', 'light.per': 4, 'light.cycle': false,
    profile: 'Default',
  };
  let wasSbx = true;
  MON.forEach(([k], i) => { S[`mm.${k}.v`] = [40, 60, 50][i]; S[`mm.${k}.m`] = [true, true, true][i]; });
  REC.forEach(([k], i) => { S[`mr.${k}.v`] = [50, 80, 50, 100][i]; S[`mr.${k}.m`] = [true, false, true, true][i]; });
  function loadCurve() {
    const p = PRESETS[S['eq.preset']]; if (!p) return;
    const c = p[S.output === 'Speakers' ? 0 : 1];
    S['eq.pre'] = c[0]; for (let i = 0; i < 10; i++) S['eq.b' + i] = c[i + 1];
  }
  for (let i = 0; i < 10; i++) S['eq.b' + i] = 0;
  loadCurve();
  try { Object.assign(S, JSON.parse(localStorage.getItem('g6mock.ui') || '{}')); } catch (e) { /* file:// without storage */ }
  S['ui.dialog'] = false; S['ui.focus'] = true; S['ui.conn'] = true; S['ui.fail'] = false;
  // a link can set the look and the page: 01-sbc-sidebar.html#theme=Light&backdrop=Acrylic&page=eq
  new URLSearchParams(location.hash.slice(1)).forEach((v, k) => { if ('ui.' + k in S) S['ui.' + k] = v; });

  const subs = [], acts = {};
  // Every change goes to the device at once: there is no preview and no apply step. `flag` is the
  // CLI flag the change stands for (the tooltip of the status line). `opt.live` marks a change made
  // while dragging: the control itself shows the value, and the status line speaks once, on release.
  // The status line names what changed, what it pulled with it (Direct mode turning SBX off), and
  // offers Undo: the settings as they were before the change, sent again at once.
  // Two ways to fail: the G6 is away (device controls are disabled, a banner says so), or a write is
  // refused (the control springs back to the device's value and a short error shows).
  const NOCONN = 'G6 not connected. Nothing was changed.', REFUSED = 'The G6 did not take the change. Nothing was changed.';
  const DEV = k => !k.startsWith('ui.');
  const LINKED = { sbx: 'SBX', scout: 'Scout Mode', direct: 'Direct mode', spdif: 'SPDIF-Out Direct' };
  let snap = null, snapT, undoRec = null, liveDirty = false;
  const restore = from => { for (const q in from) if (DEV(q)) S[q] = from[q]; };
  const valueText = (k, v) => /^eq\.(b\d|pre)$/.test(k) ? `${v > 0 ? '+' : ''}${v} dB` : k === 'mic.boost' ? `+${v} dB` : k === 'light.per' ? `${v} s` : String(v);
  function describe(k, v, label, before) {
    if (k === 'profile') return `Profile “${v}” loaded`;
    label = label || k;
    let s = typeof v !== 'boolean' ? `${label}: ${valueText(k, v)}` :
      /^Mute /.test(label) ? `${label[5].toUpperCase()}${label.slice(6)} ${v ? 'muted' : 'unmuted'}` : `${label} ${v ? 'on' : 'off'}`;
    if (k === 'output') s += S.mute ? ', muted' : `, volume ${S.vol}`;   // the switch keeps the volume: say what it now plays at
    const also = Object.keys(LINKED).filter(q => q !== k && before && before[q] !== S[q]).map(q => `${LINKED[q]} ${S[q] ? 'on' : 'off'}`);
    return also.length ? `${s} · ${also.join(', ')}` : s;
  }
  const flagText = (flag, v) => !flag ? '' : flag.endsWith('-to') ? `--${flag}-${String(v).toLowerCase().replace('.', '-')}` : `--${flag} ${typeof v === 'boolean' ? (v ? 'Enabled' : 'Disabled') : v}`;
  // the end of a drag: one status line for the whole drag
  const done = (k, flag, label) => {
    if (!liveDirty) return; liveDirty = false;
    if (S['ui.conn'] && !snap && undoRec) status(describe(k, S[k], label, undoRec.snap), flagText(flag, S[k]), false, true);
  };
  function undo() {
    if (!undoRec) return;
    if (!S['ui.conn']) return status(NOCONN, '', true);
    const r = undoRec; undoRec = null; restore(r.snap); render(); status(`Undone: ${r.label || 'the last change'}`);
  }
  function set(k, v, flag, opt = {}) {
    if (k === 'lay.cur') { const spk = S.output === 'Speakers'; k = spk ? 'lay.spk' : 'lay.hp'; flag = `playback-${spk ? 'speakers' : 'headphones'}-to`; }
    if (S[k] === v) return;
    const dev = DEV(k), before = dev ? { ...S } : null;
    if (dev && !S['ui.conn']) return status(NOCONN, '', true);   // the controls are disabled then; a stray call changes nothing
    if (dev && S['ui.fail'] && !snap) snap = before;
    S[k] = v;
    // Direct mode and SPDIF-Out Direct exclude each other (as in the Python GUI); Direct mode turns SBX and Scout Mode off
    if (k === 'spdif' && v && S.direct) { S.direct = false; S.sbx = wasSbx; }
    if (k === 'direct') { if (v) { wasSbx = S.sbx || wasSbx; S.sbx = false; S.scout = false; S.spdif = false; } else S.sbx = wasSbx; }
    if (k === 'sbx' && v) { S.scout = false; wasSbx = true; }
    if (k === 'scout' && v) { S.sbx = false; wasSbx = false; }
    if (/^eq\.(b\d|pre)$/.test(k)) S['eq.preset'] = 'Custom';
    if ((k === 'eq.preset' && v !== 'Custom') || k === 'output') loadCurve();
    if (k.startsWith('ui.')) {
      try {
        const ui = {}; for (const q of ['ui.theme', 'ui.backdrop', 'ui.corners', 'ui.wall']) ui[q] = S[q];
        localStorage.setItem('g6mock.ui', JSON.stringify(ui));
      } catch (e) { /* ignore */ }
    }
    render();
    if (!dev) return;
    if (snap) {   // the write failed: show the device's values again, and say why
      clearTimeout(snapT);
      snapT = setTimeout(() => { restore(snap); snap = null; liveDirty = false; render(); status(REFUSED, '', true); }, 350);
      return;
    }
    // one undo step per control: a drag, held arrow keys or a run through a dropdown go back to where they started
    const now = performance.now();
    if (!(undoRec && undoRec.k === k && now - undoRec.t < 2000)) undoRec = { k, snap: before };
    undoRec.t = now; undoRec.label = opt.label || undoRec.label;
    if (opt.live) liveDirty = true;
    else status(describe(k, v, opt.label, before), flagText(flag, v), false, true);
  }
  // "a|b" any, "a&b" all, "!a" not, "k=v" equals
  const term = t => {
    t = t.trim(); const neg = t[0] === '!'; if (neg) t = t.slice(1);
    const [k, want] = t.split('='); const r = want === undefined ? !!S[k] : String(S[k]) === want;
    return neg ? !r : r;
  };
  const truth = expr => expr.split('|').some(g => g.split('&').every(term));

  // ---- icons: glyph.rs paths (16 grid, stroke 1.25, round). The last block is new, same style. --
  const GLYPH = {
    close: 'M3.5 3.5l9 9M12.5 3.5l-9 9', minus: 'M3 8h10', plus: 'M3 8h10M8 3v10',
    right: 'M5.8 3.5L10.3 8l-4.5 4.5',
    warn: 'M8 2.2l6.2 10.9a.6 .6 0 0 1-.5 .9H2.3a.6 .6 0 0 1-.5-.9zM8 6.4v3.4M7.825 11.7a.175 .175 0 1 0 .35 0a.175 .175 0 1 0-.35 0z',
    info: 'M2 8a6 6 0 1 0 12 0a6 6 0 1 0-12 0zM8 7.3v3.9M7.825 4.9a.175 .175 0 1 0 .35 0a.175 .175 0 1 0-.35 0z',
    check: 'M3 8.5l3.2 3.2L13 5', reload: 'M11.9 4.1A5.5 5.5 0 1 0 13.5 8M12.5 1.5v3h-3',
    save: 'M3 2.5h8.3l2.2 2.2V13a.5 .5 0 0 1-.5 .5H3a.5 .5 0 0 1-.5-.5V3a.5 .5 0 0 1 .5-.5zM5 2.5V6h5V2.5M5 13.5v-4h6v4',
    folder: 'M1.5 11.5V4a1 1 0 0 1 1-1h3.2l1.5 1.5h5.3a1 1 0 0 1 1 1V7M3.6 7h11.2l-1.9 5.4a.9 .9 0 0 1-.85 .6H2.4a.9 .9 0 0 1-.85-1.2z',
    sliders: 'M2.5 4.5h2M8 4.5h5.5M2.5 11.5h5.5M11.5 11.5h2M4.75 4.5a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0zM8.25 11.5a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0z',
    winmin: 'M3 8.5h10', winmax: 'M4 3h8a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z', winclose: 'M3 3l10 10M13 3L3 13',
    // new for this app
    speaker: 'M2.5 6.2h2.3L8 3.5v9L4.8 9.8H2.5zM10.5 6a2.8 2.8 0 0 1 0 4M12.3 4.2a5.4 5.4 0 0 1 0 7.6',
    muted: 'M2.5 6.2h2.3L8 3.5v9L4.8 9.8H2.5zM10.8 6.2l3.2 3.6M14 6.2l-3.2 3.6',
    mic: 'M6 3.5a2 2 0 0 1 4 0v4a2 2 0 0 1-4 0zM3.5 7.5a4.5 4.5 0 0 0 9 0M8 12v2.5',
    bulb: 'M5.5 10.5a4 4 0 1 1 5 0v1.5h-5zM6.5 14.5h3',
    wave: 'M1.5 9c1.5 0 2-5 3.5-5s2 8 3.5 8 2-6 3.5-6 1.5 2 2.5 2',
    chip: 'M4.5 4.5h7v7h-7zM6.5 2v2.5M9.5 2v2.5M6.5 11.5V14M9.5 11.5V14M2 6.5h2.5M2 9.5h2.5M11.5 6.5H14M11.5 9.5H14',
    mixer: 'M4 2.5v11M8 2.5v11M12 2.5v11M2.5 5h3M6.5 10h3M10.5 6.5h3',
    chain: 'M1.5 6h3.5v4H1.5zM6.25 6h3.5v4h-3.5zM11 6h3.5v4H11zM5 8h1.25M9.75 8H11',
    spark: 'M8 1.5l1.6 4.9 4.9 1.6-4.9 1.6L8 14.5l-1.6-4.9L1.5 8l4.9-1.6z',
  };
  const svg = n => `<svg class="g" viewBox="0 0 16 16"><path d="${GLYPH[n] || ''}"/></svg>`;
  const icon = n => `<i class="ic">${svg(n)}</i>`;

  // ---- markup helpers --------------------------------------------------------------------------
  // `label` is the control's name: its aria-label, and the subject of the status line ("Surround level: 70").
  const at = o => Object.entries(o).filter(([, v]) => v !== undefined && v !== false && v !== '').map(([k, v]) => ` data-${k}="${v}"`).join('');
  const h = {
    icon, svg,
    tg: (k, flag, dis, label) => `<button class="tg" role="switch"${at({ k, flag, dis, label })}></button>`,
    // guard: a volume. A press beside the knob moves it one step towards the pointer instead of jumping there.
    sl: (k, o = {}) => `<div class="sl${o.vert ? ' vert' : ''}" role="slider" tabindex="0"${at({ k, min: o.min ?? 0, max: o.max ?? 100, step: o.step ?? 1, flag: o.flag, dis: o.dis, label: o.label, guard: o.guard, unit: o.unit })}></div>`,
    seg: (k, opts, o = {}) => `<div class="seg ${o.cls || ''}"${at({ k, opts: opts.join('|'), flag: o.flag, dis: o.dis, label: o.label })}></div>`,
    dd: (k, opts, o = {}) => `<select class="dd"${at({ k, opts: Array.isArray(opts) ? opts.join('|') : opts, flag: o.flag, dis: o.dis, label: o.label })}></select>`,
    val: (k, fmt) => `<span class="val"${at({ v: k, fmt })}></span>`,
    step: (k, o = {}) => `<span class="step"${at({ k, min: o.min, max: o.max, step: o.step || 1, unit: o.unit, dis: o.dis, label: o.label })}></span>`,
    row: (label, ctl, help, o = {}) => `<div class="row${help ? ' help' : ''}${o.child ? ' child' : ''}${o.wide ? ' wrap' : ''}"${at({ dis: o.dis, show: o.show })}><div class="lb"><span>${label}</span>${help ? `<small>${help}</small>` : ''}</div><div class="ct${o.wide ? ' wide' : ''}">${ctl}</div></div>`,
    card: (title, body, o = {}) => `<section class="card ${o.cls || ''}"${at({ dis: o.dis })}><header><h3>${title}</h3><span class="sp"></span>${o.head || ''}</header>${body}</section>`,
    // a volume with its mute: `name` is what is being set ("mic recording")
    volrow: (label, name, vk, mk, vflag, mflag, o = {}) => h.row(label, h.sl(vk, { step: o.step || 10, flag: vflag, label: `${name[0].toUpperCase()}${name.slice(1)} volume`, guard: 1 }) + h.val(vk) +
      `<span class="cap11" aria-hidden="true">Mute</span>` + h.tg(mk, mflag, '', `Mute ${name}`), '', { wide: true }),
  };
  const strip = (name, label, vk, mk, vflag, mflag, lv, o = {}) => `<div class="strip${o.master ? ' master' : ''}" role="group" aria-label="${label}" data-vk="${vk}" data-mk="${mk}" data-lv="${lv}"><span class="nm">${name}</span>
    <div class="fd"><div class="meter"></div>${h.sl(vk, { vert: 1, step: o.step || 10, flag: vflag, label: `${label} volume`, guard: 1 })}${o.stereo ? '<div class="meter" style="--sp:1.1s"></div>' : ''}</div>
    ${h.val(vk)}<button class="mb" data-tog="${mk}" data-flag="${mflag}" data-label="Mute ${label[0].toLowerCase()}${label.slice(1)}">Mute</button></div>`;
  const grp = (title, strips, extra = '') => `<section class="grp" aria-label="${title}"><h3>${title}<span class="sp"></span>${extra}</h3><div class="strips">${strips}</div></section>`;
  // why the page is dimmed, with the way out: Direct mode first, then SBX being off
  const directNotice = () => `<div class="notice" data-show="direct">${icon('warn')}<span>Direct mode is on: SBX and Scout Mode are bypassed.</span><button class="btn" data-act="direct-off">Turn off Direct mode</button></div>`;
  const sbxOffNotice = () => `<div class="notice info" data-show="!direct&!sbx">${icon('info')}<span><span data-show="scout">Scout Mode is on, so SBX is off and the effects below are not applied.</span><span data-show="!scout">SBX is off: the effects below are not applied.</span></span><button class="btn" data-act="sbx-on">Turn on SBX</button></div>`;
  const fxCard = ([id, name, flag, help]) => h.card(name,
    `<div class="row">${h.sl(`sbx.${id}.v`, { flag: flag + '-value', dis: `!sbx.${id}`, label: `${name} level` })}${h.val(`sbx.${id}.v`)}</div>` +
    (id === 'smart' ? h.row('Mode', h.seg('sbx.smart.mode', ['Normal', 'Loud', 'Night'], { cls: 'sm', flag: 'sbx-smart-volume-special-value', dis: '!sbx.smart', label: 'Smart Volume mode' })) : '') +
    `<small>${help}</small>`, { head: h.tg('sbx.' + id, flag, '', name), dis: 'direct|!sbx' });
  const eqSliders = () => `<div class="eqwrap" data-dis="!eq"><div class="col pre"><b>Pre</b>${h.sl('eq.pre', { vert: 1, min: -12, max: 12, step: .5, flag: 'eq-preamp', label: 'Preamp', unit: 'dB' })}${h.val('eq.pre', 'n')}</div>` +
    FREQS.map((f, i) => `<div class="col"><b>${f}</b>${h.sl('eq.b' + i, { vert: 1, min: -12, max: 12, step: .5, flag: 'eq-band ' + f, label: `${f.replace('k', ' k')}Hz`.replace(/(\d)Hz/, '$1 Hz'), unit: 'dB' })}${h.val('eq.b' + i, 'n')}</div>`).join('') + '</div>';
  const eqCurve = (hh = 170, drag = true) => `<svg class="eqcurve" role="img" aria-label="Equalizer curve. The band sliders below set the same values." data-c="eqcurve" data-h="${hh}"${drag ? ' data-drag="1"' : ''}></svg>`;
  const presetDD = () => h.dd('eq.preset', '@presets', { flag: 'eq-preset', label: 'Equalizer preset' });
  const lightRing = () => `<div class="ring" data-c="ring" aria-hidden="true"></div>`;
  const COLOURS = [['#ff2a2a', 'Red'], ['#ff8a1f', 'Orange'], ['#ffd400', 'Yellow'], ['#3df27a', 'Green'], ['#22d3ee', 'Cyan'], ['#3b8eea', 'Blue'], ['#a855f7', 'Purple'], ['#ffffff', 'White']];
  const swatches = () => `<div class="swatches" role="group" aria-label="Colour">${COLOURS.map(([c, n]) => `<button style="background:${c}" data-pick="light.color=${c}" data-flag="lighting-rgb" data-label="Colour" aria-label="${n}" title="${n}"></button>`).join('')}</div>`;
  const statusDL = () => `<dl class="stat"><dt>Firmware</dt><dd class="mono" data-v="firmware"></dd><dt>Output</dt><dd data-v="output"></dd><dt>Direct mode</dt><dd data-v="direct" data-fmt="onoff"></dd>
    <dt>SPDIF-Out Direct</dt><dd data-v="spdif" data-fmt="onoff"></dd><dt>Decoder mode</dt><dd data-v="decoder"></dd><dt>DAC filter</dt><dd data-v="filter"></dd>
    <dt>SBX</dt><dd data-v="sbx" data-fmt="onoff"></dd><dt>Scout Mode</dt><dd data-v="scout" data-fmt="onoff"></dd><dt>Equalizer</dt><dd data-v="eq" data-fmt="onoff"></dd>
    <dt>Mic boost</dt><dd><span data-v="mic.boost"></span> dB</dd><dt>Speaker setup</dt><dd>Speakers <span data-v="lay.spk"></span> · Headphones <span data-v="lay.hp"></span></dd><dt>Transport</dt><dd>HID if 4 · UAC via Core Audio</dd></dl>`;
  const appearanceRows = () =>
    h.row('Theme', h.seg('ui.theme', ['System', 'Light', 'Dark'])) +
    h.row('Window backdrop', h.seg('ui.backdrop', ['Off', 'Mica', 'Acrylic']), 'Windows 11. Behind the bars and the side pane.') +
    h.row('Window corners', h.seg('ui.corners', ['Square', 'Small', 'Round']));
  const LAYOUTS = ['Stereo', '5.1', '7.1'], OUTS = ['Speakers', 'Headphones'];
  // one row per output; the one that is playing now is marked
  const layRow = (out, k) => h.row(`${out} <span class="badge" data-show="output=${out}">In use</span>`,
    h.seg(k, LAYOUTS, { flag: `playback-${out.toLowerCase()}-to`, label: `Speaker setup for ${out}` }));

  const sec = {
    sbx: () => directNotice() + sbxOffNotice() + `<div class="cards">` +
      h.card('Processing mode', h.row('SBX', h.tg('sbx', 'sbx'), 'Master switch for the five effects below. Turns Scout Mode off.') +
        h.row('Scout Mode', h.tg('scout', 'scout-mode'), 'Makes footsteps and quiet cues easier to hear. Turns SBX off.'), { dis: 'direct', cls: 'span' }) +
      FX.map(fxCard).join('') + `</div>`,
    eq: () => `<div class="cards">` + h.card('Equalizer',
      h.row('Preset', presetDD() + `<button class="btn" data-act="eq-flat">Flat</button>`, `A preset has one curve per output. This is its <span data-v="output"></span> curve.`, { wide: 1 }) +
      eqCurve(150) + eqSliders(), { head: h.tg('eq', 'eq', '', 'Equalizer'), cls: 'span' }) + `</div>`,
    playback: () => `<div class="cards">` +
      h.card('Output', h.row('Output', h.seg('output', OUTS, { flag: 'set-output' }), 'Switches at once and keeps the volume.') +
        h.row('Volume', h.sl('vol', { flag: 'playback-volume', label: 'Volume', guard: 1 }) + h.val('vol') + `<span class="cap11" aria-hidden="true">Mute</span>` + h.tg('mute', 'playback-mute', '', 'Mute playback'), '', { wide: true }) +
        `<div class="subh">Speaker setup</div>` + layRow('Speakers', 'lay.spk') + layRow('Headphones', 'lay.hp') +
        `<small>Kept per output. Windows: sets the speaker setup of the G6 playback endpoint, as Sound Blaster Command does.</small>`) +
      h.card('Decoder', h.row('Dynamic range', h.seg('decoder', ['Normal', 'Full', 'Night'], { flag: 'decoder-mode' }), 'For Dolby Digital input.') +
        h.row('DAC filter', h.dd('filter', FILTERS, { flag: 'playback-filter' })) +
        `<small data-show="direct">The G6 reports no decoder mode or filter while Direct mode is on.</small>`) +
      h.card('Signal path', h.row('Direct mode', h.tg('direct', 'playback-direct-mode'), 'The shortest path to the DAC. Turns off SBX, Scout Mode and SPDIF-Out Direct.') +
        h.row('SPDIF-Out Direct', h.tg('spdif', 'playback-spdif-out-direct-mode'), 'Passes the stream to the optical output untouched. Turns off Direct mode.') +
        h.row('Headphone virtualization to Line Out', h.tg('lineout', 'playback-line-out-headphone-virtualization'), 'For Line Out: the Speakers output.'), { cls: 'span' }) + `</div>`,
    mic: () => `<div class="cards">` +
      h.card('Microphone', h.row('Mic boost (dB)', h.seg('mic.boost', ['0', '10', '20', '30'], { flag: 'recording-mic-boost-db' })) +
        h.volrow('Recording', 'mic recording', 'mic.vol', 'mic.mute', 'recording-mic-recording-volume', 'recording-mute') +
        h.volrow('Monitoring', 'mic monitoring', 'mon.vol', 'mon.mute', 'recording-mic-monitoring-volume', 'recording-mic-monitoring-mute') +
        `<small>Monitoring plays the microphone back on the output.</small>`) +
      h.card('Voice Clarity', `<div data-dis="!vc">` +
        h.row('Noise reduction', h.sl('vc.nr', { step: 20, flag: 'recording-voice-clarity-noise-reduction' }) + h.val('vc.nr'), '', { wide: true }) +
        h.row('Acoustic echo cancellation', h.tg('vc.aec', 'recording-voice-clarity-aec')) +
        h.row('Smart Volume', h.tg('vc.smart', 'recording-voice-clarity-smart-volume', '', 'Microphone Smart Volume')) +
        h.row('Mic EQ', h.tg('vc.eq', 'recording-voice-clarity-mic-eq')) +
        h.row('Mic EQ preset', h.dd('vc.eqp', Array.from({ length: 10 }, (_, i) => 'Preset ' + (i + 1)), { flag: 'recording-voice-clarity-mic-eq-preset', dis: '!vc.eq' }), '', { child: 1 }) +
        `</div>`, { head: h.tg('vc', 'recording-voice-clarity', '', 'Voice Clarity') }) + `</div>`,
    // the mixer console: channel strips with a level meter, a fader and a mute each
    mixer: () => `<div class="rack">` +
      grp('Monitoring', MON.map(([k, n, f], i) => strip(n, `${n} monitoring`, `mm.${k}.v`, `mm.${k}.m`, `mixer-monitoring-${f}-volume`, `mixer-monitoring-${f}-mute`, 45 + i * 12)).join('') +
        strip('Mic', 'Mic monitoring', 'mon.vol', 'mon.mute', 'recording-mic-monitoring-volume', 'recording-mic-monitoring-mute', 60)) +
      grp('Recording sources', REC.map(([k, n, f], i) => strip(n, `${n} recording`, `mr.${k}.v`, `mr.${k}.m`, `mixer-recording-${f}-volume`, `mixer-recording-${f}-mute`, 50 + i * 10)).join('') +
        strip('Mic', 'Mic recording', 'mic.vol', 'mic.mute', 'recording-mic-recording-volume', 'recording-mute', 70)) +
      grp('Playback', strip(`<span data-v="output"></span>`, 'Playback', 'vol', 'mute', 'playback-volume', 'playback-mute', 90, { step: 1, stereo: 1, master: 1 }),
        `<span class="cap11" aria-hidden="true">Mute mix</span>` + h.tg('mix.play.mute', 'mixer-playback-mute', '', 'Mute playback mix')) + `</div>
      <div class="card inserts"><div class="ins"><span>Output</span>${h.seg('output', OUTS, { cls: 'sm', flag: 'set-output', label: 'Output' })}</div>
        <div class="ins"><span>Speaker setup</span>${h.seg('lay.cur', LAYOUTS, { cls: 'sm', label: 'Speaker setup' })}</div>
        <div class="ins"><span>Mic boost (dB)</span>${h.seg('mic.boost', ['0', '10', '20', '30'], { cls: 'sm', flag: 'recording-mic-boost-db', label: 'Mic boost (dB)' })}</div>
        <div class="ins"><span>Direct mode</span>${h.tg('direct', 'playback-direct-mode', '', 'Direct mode')}</div></div>
      <small>Levels and mutes are the Windows ones (Core Audio). A muted strip shows no level. Drag a fader's knob, or press above or below it to move one step: a press never jumps the level.</small>`,
    light: () => `<div class="cards">` +
      h.card('Lighting', `<div style="display:flex;gap:16px;align-items:center;padding:8px 0">${lightRing()}<div style="flex:1;min-width:0"><div data-dis="!light">` +
        h.row('Colour', `<span class="mono muted" data-v="light.color"></span><input type="color" data-k="light.color" data-flag="lighting-rgb" data-label="Colour">`) + swatches() + `</div></div></div>` +
        h.row('Volume ring light', h.tg('light.ring', 'lighting-volume-ring'), 'The LED indicator round the knob.'), { head: h.tg('light', '', '', 'Lighting') }) +
      h.card('Effects', h.row('Effect', h.seg('light.fx', ['Off', 'Pulsate', 'Cycle', 'Music'], { flag: 'lighting-effect', cls: 'sm', label: 'Lighting effect' })) +
        h.row('Period', h.step('light.per', { min: 1, max: 30, unit: ' s', dis: 'light.fx=Off' }), 'Seconds for one pulse or colour turn. Needs an effect.') +
        h.row('Music: cycle colours', h.tg('light.cycle', '', '!light.fx=Music'), 'Needs the Music effect.') +
        `<small>Host-side effects: they run while this app is open. Music follows the playback level (Windows).</small>`, { dis: '!light' }) + `</div>`,
    system: () => `<div class="cards">` +
      h.card('Profiles', `<div class="plist" role="group" aria-label="Load a profile">${['Default', 'Gaming (headphones)', 'Late night'].map(p => `<button data-pick="profile=${p}">${icon('save')}<span>${p}</span></button>`).join('')}</div>
        <div class="row"><button class="btn" data-act="profile-load">${icon('folder')}Load…</button><button class="btn" data-act="profile-save">${icon('save')}Save…</button></div>
        <small>Picking a profile loads it at once: every setting it stores changes together. Undo brings the earlier settings back. Not stored: output, RGB colour, volumes, speaker setup.</small>`) +
      h.card('Device status', statusDL(), { head: `<button class="btn ico" data-act="refresh" title="Refresh" aria-label="Read the status again">${icon('reload')}</button>` }) +
      h.card('Appearance', appearanceRows(), { cls: 'span' }) + `</div>`,
  };

  const comp = {
    titlebar: el => `<span class="mark"></span><span class="ttl">SoundBlaster X G6</span><span class="sub">${el.dataset.sub || ''}</span><span class="badge err" data-show="!ui.conn">Not connected</span><span class="sp"></span>
      <button class="tbtn" data-act="settings" title="Appearance: theme, window backdrop, window corners" aria-label="Appearance settings">${icon('sliders')}</button>
      <span class="cap"><button aria-label="Minimize">${svg('winmin')}</button><button aria-label="Maximize">${svg('winmax')}</button><button class="x" aria-label="Close">${svg('winclose')}</button></span>`,
    // the volume stays adjustable while muted, so it can be turned down before the sound comes back
    master: () => `<button class="btn ico quiet" data-tog="mute" data-flag="playback-mute" data-label="Mute playback" title="Mute"><span data-show="!mute">${icon('speaker')}</span><span data-show="mute">${icon('muted')}</span></button>${h.sl('vol', { flag: 'playback-volume', label: 'Volume', guard: 1 })}<span class="val" data-show="!mute" data-v="vol"></span><span class="val" data-show="mute">Muted</span>`,
  };

  // ---- EQ curve ------------------------------------------------------------------------------------
  // Drawn at the element's own width (redrawn when that changes), so the labels and the points keep their size at 380 px.
  const geo = el => { const W = el._W || 600, PADX = W < 420 ? 24 : 34; return { W, PADX, bx: i => PADX + i * (W - 2 * PADX) / 9, edge: W < 420 ? 6 : 14 }; };
  function curveInit(el) {
    el._W = Math.max(260, Math.round(el.clientWidth) || 600);
    const H = +el.dataset.h, y = db => 12 + (12 - db) / 24 * (H - 34), { W, PADX, bx, edge } = geo(el);
    el.setAttribute('viewBox', `0 0 ${W} ${H}`);
    let s = '';
    for (const db of [12, 6, -6, -12]) s += `<line class="grid" x1="${PADX - edge}" x2="${W - PADX + edge}" y1="${y(db)}" y2="${y(db)}"/><text class="lbl" x="0" y="${y(db) + 3.5}">${db > 0 ? '+' : ''}${db}</text>`;
    s += `<line class="zero" x1="${PADX - edge}" x2="${W - PADX + edge}" y1="${y(0)}" y2="${y(0)}"/><path class="area"/><path class="crv"/>`;
    FREQS.forEach((f, i) => { s += `<text class="lbl" text-anchor="middle" x="${bx(i)}" y="${H - 4}">${f}</text>`; });
    const col = (W - 2 * PADX) / 9;
    if (el.dataset.drag) FREQS.forEach((f, i) => { s += `<circle class="pt" r="6.5" cx="${bx(i)}" data-b="${i}"/><rect x="${bx(i) - col / 2}" y="0" width="${col}" height="${H - 18}" fill="transparent" data-b="${i}" style="cursor:ns-resize"/>`; });
    el.innerHTML = s;
    if (!el.dataset.drag || el._wired) return;
    el._wired = true;
    let band = null;
    const label = () => FREQS[band].replace('k', ' k') + (FREQS[band].endsWith('k') ? 'Hz' : ' Hz');
    const move = e => {
      const py = new DOMPoint(e.clientX, e.clientY).matrixTransform(el.getScreenCTM().inverse()).y;
      const db = Math.max(-12, Math.min(12, Math.round((12 - (py - 12) / (H - 34) * 24) * 2) / 2));
      set('eq.b' + band, db, 'eq-band ' + FREQS[band], { live: true, label: label() });
    };
    el.addEventListener('pointerdown', e => {
      const b = e.target.dataset && e.target.dataset.b; if (b === undefined || !S.eq || !S['ui.conn']) return;
      band = +b; el.setPointerCapture(e.pointerId); el.querySelector(`circle[data-b="${band}"]`).classList.add('drag'); move(e);
    });
    el.addEventListener('pointermove', e => { if (band !== null) move(e); });
    const end = () => { if (band !== null) { el.querySelector('.pt.drag')?.classList.remove('drag'); done('eq.b' + band, 'eq-band ' + FREQS[band], label()); band = null; } };
    el.addEventListener('pointerup', end); el.addEventListener('pointercancel', end);
    new ResizeObserver(() => { const w = Math.round(el.clientWidth); if (w && Math.abs(w - el._W) > 1 && band === null) { curveInit(el); curveDraw(el); } }).observe(el);
  }
  function curveDraw(el) {
    const H = +el.dataset.h, y = db => 12 + (12 - db) / 24 * (H - 34), { W, PADX, bx, edge } = geo(el);
    const pts = FREQS.map((_, i) => [bx(i), y(Math.max(-12, Math.min(12, S['eq.b' + i] + S['eq.pre'])))]);
    const P = [[PADX - edge, pts[0][1]], ...pts, [W - PADX + edge, pts[9][1]]];
    let d = `M${P[0][0]} ${P[0][1]}`;
    for (let i = 0; i < P.length - 1; i++) {
      const p0 = P[Math.max(0, i - 1)], p1 = P[i], p2 = P[i + 1], p3 = P[Math.min(P.length - 1, i + 2)];
      d += `C${(p1[0] + (p2[0] - p0[0]) / 6).toFixed(1)} ${(p1[1] + (p2[1] - p0[1]) / 6).toFixed(1)} ${(p2[0] - (p3[0] - p1[0]) / 6).toFixed(1)} ${(p2[1] - (p3[1] - p1[1]) / 6).toFixed(1)} ${p2[0]} ${p2[1]}`;
    }
    el.querySelector('.crv').setAttribute('d', d);
    el.querySelector('.area').setAttribute('d', `${d}L${W - PADX + edge} ${y(0)}L${PADX - edge} ${y(0)}Z`);
    el.querySelectorAll('.pt').forEach((c, i) => c.setAttribute('cy', pts[i][1]));
    el.classList.toggle('bypass', !S.eq || !S['ui.conn']);
  }

  // ---- binding -------------------------------------------------------------------------------------
  const fmt = (v, f) => f === 'onoff' ? (v ? 'On' : 'Off') : f === 'db' ? `${v > 0 ? '+' : ''}${v} dB` : f === 'n' ? `${v > 0 ? '+' : ''}${v}` : f === 'pct' ? v + '%' : v;
  const $$ = (q, r = document) => [...r.querySelectorAll(q)];
  // a control's name: its own, or the one of its group (segments, stepper)
  const lab = el => el.getAttribute('aria-label') && !el.dataset.o && !el.dataset.pick && !el.dataset.d ? el.getAttribute('aria-label') : el.dataset.label || el.parentElement.getAttribute('aria-label') || '';
  // The status line: what changed, in words, and Undo. Errors in the warn colours, announced at once.
  let statusT, statusMs = 0;
  const statusArm = () => { clearTimeout(statusT); statusT = setTimeout(() => document.querySelector('.status')?.classList.remove('show'), statusMs); };
  function status(text, detail, err, canUndo) {
    const t = document.querySelector('.status'); if (!t) return;
    const [ic, ok, bad, btn] = t.children;
    ic.innerHTML = svg(err ? 'warn' : 'check'); ok.textContent = err ? '' : text; bad.textContent = err ? text : '';
    btn.hidden = !canUndo; t.title = detail || '';
    t.classList.toggle('err', !!err); t.classList.add('show');
    statusMs = err ? 6000 : canUndo ? 5000 : 1800; statusArm();
  }
  const clamp = (d, v) => +Math.max(+d.min, Math.min(+d.max, v)).toFixed(2);
  let grab = 0;
  function sliderSet(el, e) {
    const d = el.dataset, r = el.getBoundingClientRect(), min = +d.min, max = +d.max, step = +d.step;
    let f = el.classList.contains('vert') ? 1 - (e.clientY - grab - r.top - 7) / (r.height - 14) : (e.clientX - grab - r.left - 7) / (r.width - 14);
    f = Math.max(0, Math.min(1, f));
    set(d.k, +(Math.round((min + f * (max - min)) / step) * step).toFixed(2), d.flag, { live: true, label: lab(el) });
  }
  const bigStep = d => Math.max(+d.step, Math.round((+d.max - +d.min) / 10 / +d.step) * +d.step);
  function build(root) {
    $$('[data-sec]', root).forEach(el => { el.innerHTML = sec[el.dataset.sec](); });
    $$('[data-c]', root).forEach(el => { const f = comp[el.dataset.c]; if (f) { el.innerHTML = f(el); } });
    $$('i[data-g]', root).forEach(el => { if (!el.children.length) { el.classList.add('ic'); el.innerHTML = svg(el.dataset.g); } });
    $$('.seg[data-opts]', root).forEach(el => { if (!el.children.length) el.innerHTML = el.dataset.opts.split('|').map(o => `<button data-o="${o}">${o}</button>`).join(''); });
    $$('select.dd[data-opts]', root).forEach(el => {
      if (el.options.length) return;
      const opts = el.dataset.opts === '@presets' ? ['Custom', ...Object.keys(PRESETS)] : el.dataset.opts.split('|');
      el.innerHTML = opts.map(o => `<option>${o}</option>`).join('');
    });
    $$('.sl', root).forEach(el => { if (!el.children.length) el.innerHTML = `<div class="trk"><div class="fil"></div></div><div class="knb"></div>`; });
    $$('.step[data-k]', root).forEach(el => { if (!el.children.length) el.innerHTML = `<button data-d="-1" aria-label="Less">${svg('minus')}</button><output></output><button data-d="1" aria-label="More">${svg('plus')}</button>`; });
    $$('svg[data-c=eqcurve]', root).forEach(el => { if (!el.children.length) curveInit(el); });
    // names for assistive technology: the given label, else the row's label, else the card's title
    $$('.tg,.sl,.seg,select.dd,.step,input[data-k],[data-tog],[data-pick]', root).forEach(el => {
      if (el.matches('.seg,.step')) el.setAttribute('role', 'group');
      if (el.matches('[data-pick],[data-tog]') && !el.dataset.label) return;   // named by their own text
      if (el.getAttribute('aria-label')) { if (el.dataset.label === undefined) el.dataset.label = el.getAttribute('aria-label'); return; }
      const row = el.closest('.row'), rl = row && row.querySelector('.lb>span'), card = el.closest('.card'), ct = card && card.querySelector('header h3');
      const name = el.dataset.label || (rl && rl.firstChild.textContent.trim()) || (ct && ct.textContent.trim()) || '';
      if (name) el.setAttribute('aria-label', name);
    });
    $$('.sl.vert', root).forEach(el => el.setAttribute('aria-orientation', 'vertical'));
    $$('.meter,i.ic,.led,.wire,.mark,.dot', root).forEach(el => el.setAttribute('aria-hidden', 'true'));
  }
  // what a control writes to: a device setting (off while the G6 is away) or a setting of the app itself
  const DEVACTS = ['direct-off', 'sbx-on', 'eq-flat', 'profile-load', 'refresh'];
  function isDev(el) {
    const d = el.dataset, p = el.parentElement ? el.parentElement.dataset : {};
    const k = d.k || d.tog || (d.pick && d.pick.split('=')[0]) || ((d.o !== undefined || d.d) && p.k) || '';
    return k ? DEV(k) : DEVACTS.includes(d.act);
  }
  function render() {
    const H = document.documentElement;
    const theme = S['ui.theme'] === 'System' ? (matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark') : S['ui.theme'].toLowerCase();
    H.dataset.theme = theme;
    // frost.rs: the chrome is frosted only while the setting names a backdrop and the window has the keyboard
    H.dataset.frost = S['ui.focus'] ? S['ui.backdrop'].toLowerCase() : 'off';
    H.dataset.focus = S['ui.focus'] ? 'yes' : 'no';
    H.dataset.corners = S['ui.corners']; H.dataset.wall = S['ui.wall']; H.dataset.conn = S['ui.conn'] ? 'yes' : 'no';
    S['lay.cur'] = S[S.output === 'Speakers' ? 'lay.spk' : 'lay.hp'];
    $$('.tg[data-k]').forEach(el => el.setAttribute('aria-checked', !!S[el.dataset.k]));
    $$('.seg[data-k]').forEach(el => $$('button', el).forEach(b => b.setAttribute('aria-pressed', String(S[el.dataset.k]) === b.dataset.o)));
    $$('select[data-k]').forEach(el => { el.value = S[el.dataset.k]; });
    $$('input[data-k]').forEach(el => { if (document.activeElement !== el) el.value = S[el.dataset.k]; });
    $$('.sl[data-k]').forEach(el => {
      const d = el.dataset, min = +d.min, max = +d.max, v = +S[d.k], f = (v - min) / (max - min), z = min < 0 ? -min / (max - min) : 0;
      const a = Math.min(f, z) * 100, b = Math.abs(f - z) * 100, vert = el.classList.contains('vert');
      const fil = el.firstChild.firstChild, knb = el.lastChild;
      if (vert) { fil.style.bottom = a + '%'; fil.style.height = b + '%'; knb.style.bottom = `calc(7px + ${f} * (100% - 14px))`; }
      else { fil.style.left = a + '%'; fil.style.width = b + '%'; knb.style.left = `calc(7px + ${f} * (100% - 14px))`; }
      el.setAttribute('aria-valuenow', v); el.setAttribute('aria-valuemin', min); el.setAttribute('aria-valuemax', max);
      if (d.unit) el.setAttribute('aria-valuetext', `${v > 0 && min < 0 ? '+' : ''}${v} ${d.unit}`);
    });
    $$('.step[data-k]').forEach(el => { el.children[1].textContent = S[el.dataset.k] + (el.dataset.unit || ''); });
    $$('[data-v]').forEach(el => { el.textContent = fmt(S[el.dataset.v], el.dataset.fmt); });
    $$('[data-pick]').forEach(el => { const [k, v] = el.dataset.pick.split('='); el.setAttribute('aria-pressed', String(S[k]) === v); });
    $$('[data-tog]').forEach(el => el.setAttribute('aria-pressed', !!S[el.dataset.tog]));
    $$('[data-dis]').forEach(el => el.classList.toggle('off', truth(el.dataset.dis)));
    // Disabled for real, not only dimmed: out of the tab order and announced as unavailable.
    // That is a control under a switched-off parent, and every device control while the G6 is away.
    $$('.win button,.win select,.win input,.win .sl').forEach(el => {
      const dis = !!el.closest('.off') || (!S['ui.conn'] && isDev(el));
      if (el.classList.contains('sl')) { el.tabIndex = dis ? -1 : 0; if (dis) el.setAttribute('aria-disabled', 'true'); else el.removeAttribute('aria-disabled'); }
      else if (el.disabled !== dis) el.disabled = dis;
    });
    $$('[data-show]').forEach(el => { el.hidden = !truth(el.dataset.show); });
    $$('[data-lit]').forEach(el => { const on = truth(el.dataset.lit); el.classList.toggle('lit', on); if (el.dataset.name) { el.setAttribute('role', 'img'); el.setAttribute('aria-label', `${el.dataset.name} ${on ? 'on' : 'off'}`); } });
    $$('.nav [data-nav]').forEach(el => { if (S['ui.page'] === el.dataset.nav) el.setAttribute('aria-current', 'page'); else el.removeAttribute('aria-current'); });
    $$('.pg[data-page]').forEach(el => el.classList.toggle('cur', S['ui.page'] === el.dataset.page));
    $$('.dot').forEach(el => { el.title = S['ui.conn'] ? 'Connected' : 'Not connected'; });
    $$('svg[data-c=eqcurve]').forEach(curveDraw);
    $$('.ring[data-c=ring]').forEach(el => {
      el.style.setProperty('--c', S['light.color']); el.style.setProperty('--lit', S.light ? 1 : 0); el.style.setProperty('--per', S['light.per'] + 's');
      el.className = el.className.replace(/ ?fx-\w+/g, '') + (S.light && S['light.fx'] !== 'Off' ? ' fx-' + S['light.fx'] : '');
    });
    // meters: sample movement scaled by the fader; none when the strip is muted or the G6 is away
    $$('.strip[data-vk]').forEach(el => {
      const d = el.dataset, quiet = S[d.mk] || !S['ui.conn'] || (d.vk === 'vol' && S['mix.play.mute']);
      el.classList.toggle('muted', !!quiet); el.style.setProperty('--lv', (quiet ? 0 : d.lv * S[d.vk] / 100) + '%');
    });
    // the dialog is modal: the window behind it is out of reach while it is open
    const dlg = document.getElementById('g6dlg');
    if (dlg && dlg.hidden === S['ui.dialog']) {
      dlg.hidden = !S['ui.dialog'];
      $$('.win>.titlebar,.win>.body').forEach(el => { el.inert = S['ui.dialog']; });
      if (S['ui.dialog']) { dlg._back = document.activeElement; dlg.querySelector('[data-act=settings-close]').focus(); }
      else if (dlg._back && dlg._back.focus) dlg._back.focus();
    }
    subs.forEach(f => f(S));
  }
  function init() {
    // the settings dialog (pages.rs: card on a dimmed window, 40 px header with a x, list at the left)
    const win = document.querySelector('.win');
    if (win && !document.getElementById('g6dlg')) {
      win.insertAdjacentHTML('beforeend', `<div id="g6dlg" hidden style="position:absolute;inset:0;z-index:20;background:var(--dim);display:grid;place-items:center;padding:6px">
        <div class="card" role="dialog" aria-modal="true" aria-label="Settings" style="width:min(620px,100%);padding:0;box-shadow:0 3px 14px rgb(0 0 0/.3);max-height:100%;overflow:auto">
          <header style="margin:0;padding:0 12px 0 24px"><h3 style="font-size:17px">Settings</h3><span class="sp"></span><button class="btn ico quiet" data-act="settings-close" aria-label="Close">${icon('close')}</button></header>
          <div style="display:flex;min-height:230px;flex-wrap:wrap"><div class="nav dnav" style="border-right:1px solid var(--popup-line);flex:none"><button data-pick="ui.dlg=Appearance">Appearance</button><button data-pick="ui.dlg=Device">Device</button></div>
          <div style="flex:1;min-width:250px;padding:8px 24px 16px"><div data-show="ui.dlg=Appearance">${appearanceRows()}<div class="gap"></div><small>The toolkit's own settings (mdshared-ui). <b>Window backdrop</b> is its only transparency setting; the chrome is frosted only while the window has the keyboard.</small></div><div data-show="ui.dlg=Device" style="padding-top:8px">${statusDL()}</div></div></div></div></div>`);
      // the status line belongs to the window: bottom centre of the page area
      win.insertAdjacentHTML('beforeend', `<div class="status"><i class="ic" aria-hidden="true"></i><span role="status"></span><span role="alert"></span><button class="btn" data-act="undo" title="Undo (Ctrl+Z)" hidden>Undo</button></div>`);
      const st = win.querySelector('.status');
      const ub = st.lastChild; ub.addEventListener('pointerenter', () => clearTimeout(statusT)); ub.addEventListener('pointerleave', statusArm);
      st.addEventListener('focusin', () => clearTimeout(statusT)); st.addEventListener('focusout', statusArm);
    }
    if (!document.querySelector('.rig')) document.body.insertAdjacentHTML('beforeend', `<div class="rig"><b>Mockup</b>
      <div class="ins">Theme ${h.seg('ui.theme', ['System', 'Light', 'Dark'], { cls: 'sm', label: 'Theme' })}</div>
      <div class="ins">Window backdrop ${h.seg('ui.backdrop', ['Off', 'Mica', 'Acrylic'], { cls: 'sm', label: 'Window backdrop' })}</div>
      <div class="ins">Window corners ${h.seg('ui.corners', ['Square', 'Small', 'Round'], { cls: 'sm', label: 'Window corners' })}</div>
      <label title="Not a toolkit setting. The toolkit shows the backdrop only while the window has the keyboard.">Window focused ${h.tg('ui.focus', '', '', 'Window focused')}</label>
      <label title="Mockup only. Off shows the app without the G6: a banner, and the device controls disabled.">G6 connected ${h.tg('ui.conn', '', '', 'G6 connected')}</label>
      <label title="Mockup only. On, then change something: the control springs back and an error shows.">Writes fail ${h.tg('ui.fail', '', '', 'Writes fail')}</label>
      <div class="ins" title="Not a toolkit setting: the sample desktop behind the window.">Wallpaper ${h.seg('ui.wall', ['0', '1', '2'], { cls: 'sm', label: 'Wallpaper' })}</div>
      <span class="sp"></span><a href="index.html">About this mockup</a></div>`);
    build(document);
    if (!S['ui.page']) { const n = document.querySelector('[data-nav]:not([data-act])'); if (n) S['ui.page'] = n.dataset.nav; }

    document.addEventListener('click', e => {
      const t = e.target.closest('button,[data-act]'); if (!t || t.disabled || t.closest('.off')) return;
      const d = t.dataset, label = lab(t);
      if (t.classList.contains('tg') && d.k) return set(d.k, !S[d.k], d.flag, { label });
      if (d.o !== undefined && t.parentElement.dataset.k) return set(t.parentElement.dataset.k, d.o, t.parentElement.dataset.flag, { label });
      if (d.tog) set(d.tog, !S[d.tog], d.flag, { label });
      if (d.pick) { const [k, v] = d.pick.split('='); set(k, v, d.flag, { label }); }
      if (d.d && t.parentElement.dataset.k) { const p = t.parentElement.dataset; return set(p.k, Math.max(+p.min, Math.min(+p.max, S[p.k] + (+d.d) * (+p.step))), p.flag, { label }); }
      if (d.nav) {
        if (document.querySelector(`.pg[data-page="${d.nav}"]`)) set('ui.page', d.nav);
      }
      if (d.act && acts[d.act]) acts[d.act](t);
    });
    document.addEventListener('change', e => {
      const t = e.target; if (!t.dataset.k) return;
      if (t.type === 'color') return done(t.dataset.k, t.dataset.flag, lab(t));   // the picker closed: the final value
      if (t.tagName === 'SELECT') set(t.dataset.k, t.value, t.dataset.flag, { label: lab(t) });
    });
    document.addEventListener('input', e => { const t = e.target; if (t.type === 'color' && t.dataset.k) set(t.dataset.k, t.value, t.dataset.flag, { live: true, label: lab(t) }); });
    let drag = null;
    document.addEventListener('pointerdown', e => {
      const el = e.target.closest('.sl'); if (!el || el.getAttribute('aria-disabled') === 'true') return;
      el.focus({ focusVisible: false }); e.preventDefault(); grab = 0;
      if (el.dataset.guard) {
        // A volume never jumps to where the pointer lands: the knob is dragged from where it is,
        // and a press beside it moves one step towards the pointer.
        const k = el.lastChild.getBoundingClientRect(), vert = el.classList.contains('vert'), d = el.dataset;
        const pos = vert ? e.clientY : e.clientX, c = vert ? k.top + k.height / 2 : k.left + k.width / 2;
        if (Math.abs(pos - c) > 13) return set(d.k, clamp(d, +S[d.k] + ((vert ? pos < c : pos > c) ? 1 : -1) * bigStep(d)), d.flag, { label: lab(el) });
        grab = pos - c;
      }
      drag = el; el.classList.add('drag'); el.setPointerCapture(e.pointerId); sliderSet(el, e);
    });
    document.addEventListener('pointermove', e => { if (drag) sliderSet(drag, e); });
    const up = () => { if (drag) { drag.classList.remove('drag'); done(drag.dataset.k, drag.dataset.flag, lab(drag)); drag = null; } };
    document.addEventListener('pointerup', up); document.addEventListener('pointercancel', up);
    document.addEventListener('keydown', e => {
      const el = e.target.closest && e.target.closest('.sl');
      if (el && el.getAttribute('aria-disabled') !== 'true' && !e.ctrlKey && !e.altKey && !e.metaKey) {
        // arrows: one step. Page keys: a tenth of the range. Home: the minimum. End: the maximum, but not for a volume.
        const d = el.dataset, big = bigStep(d), v = +S[d.k];
        const to = { ArrowRight: v + +d.step, ArrowUp: v + +d.step, ArrowLeft: v - +d.step, ArrowDown: v - +d.step, PageUp: v + big, PageDown: v - big, Home: +d.min, End: d.guard ? undefined : +d.max }[e.key];
        if (to !== undefined) { e.preventDefault(); e.stopPropagation(); set(d.k, clamp(d, to), d.flag, { label: lab(el) }); return; }
      }
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === 'z') { e.preventDefault(); undo(); return; }
      if (e.key === 'Escape' && S['ui.dialog']) set('ui.dialog', false);
    }, true);
    matchMedia('(prefers-color-scheme: light)').addEventListener('change', render);
    render();
  }
  Object.assign(acts, {
    'direct-off': () => set('direct', false, 'playback-direct-mode', { label: 'Direct mode' }),
    'sbx-on': () => set('sbx', true, 'sbx', { label: 'SBX' }),
    'eq-flat': () => set('eq.preset', 'Flat', 'eq-preset', { label: 'Equalizer preset' }),
    undo,
    refresh: () => status('Status read from the G6', 'firmware ' + S.firmware),
    reconnect: () => status('G6 not found. Check the USB cable.', '', true),
    'profile-save': () => status('Profile saved', 'g6-profile.json'),
    'profile-load': () => status('Profile loaded from file', '--profile-load'),
    settings: () => set('ui.dialog', true), 'settings-close': () => set('ui.dialog', false),
  });
  document.addEventListener('DOMContentLoaded', init);
  return { S, set, on: f => subs.push(f), h, FREQS, FX, REC };
})();
