/* global React, ReactDOM, RULAKE, RuScreens, useTweaks, TweaksPanel, TweakSection, TweakRadio, TweakToggle, TweakSlider, TweakColor */
const { useState, useEffect, useMemo } = React;
const {
  Sidebar, Topbar, Statusbar,
  StatsScreen, PlaygroundScreen, BrowseScreen, BundleScreen, AuditScreen, ConnectScreen,
  AppStoreScreen,
} = RuScreens;
const { DebugConsole } = window.RuDebug;
const { WelcomeModal } = window.RuWelcome;
const { ToastHost } = window.RuToast;
const { HelpIndexModal, HelpModal, HelpHotkey } = window.RuHelp;

const hexToRgb = (hex) => {
  const h = hex.replace('#', '');
  const v = h.length === 3 ? h.split('').map(c => c+c).join('') : h;
  const n = parseInt(v, 16);
  return `${(n>>16)&255}, ${(n>>8)&255}, ${n&255}`;
};

// Generate live data — animated, evolving each tick
function useLiveStats() {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick(t => t + 1), 1100);
    return () => clearInterval(id);
  }, []);

  return useMemo(() => {
    const r = RULAKE.mulberry32(tick + 1);
    const wave = (n, base, amp, phase = 0) =>
      Array.from({ length: n }, (_, i) => Math.max(0, base + Math.sin((i + tick) * 0.4 + phase) * amp + (r() - 0.5) * amp * 0.4));

    const hits = 124_330 + tick * 23 + Math.floor(r()*8);
    const misses = 1_482 + Math.floor(tick / 4);
    const hitRate = ((hits / (hits + misses)) * 100).toFixed(2);
    const verified = hits;

    return {
      hits, misses, hitRate,
      primes: 642 + Math.floor(tick/3),
      avgPrime: 8.4,
      refused: 92,
      verified,
      delta: { hits: 1422 + Math.floor(r()*40), misses: 12 },
      qps: (28 + r() * 6).toFixed(1),
      p50: (8 + r()*2).toFixed(1),
      p99: (24 + r()*8).toFixed(1),
      spark: {
        hits:    wave(40, 30, 8, 0),
        misses:  wave(40, 4, 3, 1),
        hitRate: wave(40, 95, 1.2, 2),
        primes:  wave(40, 12, 5, 1.5),
        refused: wave(40, 6, 4, 3),
        verified: wave(40, 30, 8, 0),
      },
      chart: {
        prod: wave(40, 22, 8, 0),
        eu:   wave(40, 9, 4, 1.4),
        edge: wave(40, 2, 1.5, 2.7),
      },
      histo: [3, 14, 38, 62, 84, 96, 88, 64, 42, 28, 18, 12, 8, 5, 4, 3, 2, 2, 1, 1],
    };
  }, [tick]);
}

function App() {
  const [route, setRoute] = useState('stats');
  const [envTab, setEnvTab] = useState('prod');
  const [bundleSelection, setBundleSelection] = useState({ backend: 'lake-prod', collection: 'memories' });
  const [mobileNav, setMobileNav] = useState(false);
  const [welcomeOpen, setWelcomeOpen] = useState(false);
  const [helpIndexOpen, setHelpIndexOpen] = useState(false);
  const [helpTopic, setHelpTopic] = useState(null); // routed help topic
  const liveData = useLiveStats();

  // Global help dispatcher — any HelpIcon dispatches 'rulake:open-help' { topic }
  useEffect(() => {
    const onHelp = (e) => setHelpTopic(e.detail?.topic || null);
    const onIndex = () => setHelpIndexOpen(true);
    window.addEventListener('rulake:open-help', onHelp);
    window.addEventListener('rulake:open-help-index', onIndex);
    return () => {
      window.removeEventListener('rulake:open-help', onHelp);
      window.removeEventListener('rulake:open-help-index', onIndex);
    };
  }, []);

  // First-run welcome — read once on mount
  useEffect(() => {
    if (!window.RuStore) return;
    window.RuStore.getKV('welcome_seen').then(seen => { if (!seen) setWelcomeOpen(true); });
  }, []);

  const tweaksDefaults = /*EDITMODE-BEGIN*/{
    "accent": "verifier-green",
    "density": "terminal",
    "showVectorField": true,
    "motionSpeed": 1.0,
    "monoFont": "JetBrains Mono",
    "showConsole": false
  }/*EDITMODE-END*/;
  const [tweaks, setTweak] = useTweaks(tweaksDefaults);

  // Apply theme preset — accent now changes background, surfaces, accent, secondary, font feel
  useEffect(() => {
    const root = document.documentElement;
    const themes = {
      'verifier-green': {
        // Default — deep terminal slate, forest verifier green, amber refusals
        ink:        '#0a0e14',
        ink2:       '#11161e',
        ink3:       '#1a212c',
        ink4:       '#242c39',
        rule:       '#2a3340',
        fg:         '#d6dae0',
        fgDim:      '#9098a4',
        fgFaint:    '#6b7280',
        verifier:   '#1e5b3d',
        verifierBr: '#2d8c5d',
        refused:    '#c87b2c',
        refusedBr:  '#e89548',
        cyan:       '#4a9eb5',
        paperBg:    '#f4ecd8',
        paperFg:    '#2a2620',
        font:       "'Inter', -apple-system, system-ui, sans-serif",
      },
      'cool-teal': {
        // Calm low-stim — soft midnight, teal accent, muted desaturated refusal
        ink:        '#0b1316',
        ink2:       '#121d22',
        ink3:       '#1b2930',
        ink4:       '#27383f',
        rule:       '#2a3a42',
        fg:         '#dde4e5',
        fgDim:      '#94a3a6',
        fgFaint:    '#6e7d80',
        verifier:   '#0f5b5b',
        verifierBr: '#2dabab',
        refused:    '#a07550',
        refusedBr:  '#d29a6a',
        cyan:       '#5fb6c4',
        paperBg:    '#eef2ec',
        paperFg:    '#1f2826',
        font:       "'Inter', -apple-system, system-ui, sans-serif",
      },
      'iris': {
        // Editorial — warmer near-black, iris accent, magenta refusals
        ink:        '#0d0c14',
        ink2:       '#15131e',
        ink3:       '#1f1d2c',
        ink4:       '#2c2a3c',
        rule:       '#2f2c40',
        fg:         '#e2dfea',
        fgDim:      '#9d97b0',
        fgFaint:    '#6c6885',
        verifier:   '#3b3a8c',
        verifierBr: '#7a78d6',
        refused:    '#a3508a',
        refusedBr:  '#d678b3',
        cyan:       '#9c7ad6',
        paperBg:    '#f0ecf6',
        paperFg:    '#252135',
        font:       "'Inter', -apple-system, system-ui, sans-serif",
      },
    };
    const t = themes[tweaks.accent] || themes['verifier-green'];
    const set = (k, v) => root.style.setProperty(k, v);
    set('--ink', t.ink);
    set('--ink-2', t.ink2);
    set('--ink-3', t.ink3);
    set('--ink-4', t.ink4);
    set('--rule', t.rule);
    set('--fg', t.fg);
    set('--fg-dim', t.fgDim);
    set('--fg-faint', t.fgFaint);
    set('--verifier', t.verifier);
    set('--verifier-bright', t.verifierBr);
    set('--refused', t.refused);
    set('--refused-bright', t.refusedBr);
    set('--accent-cyan', t.cyan);
    set('--paper', t.paperBg);
    set('--paper-ink', t.paperFg);
    set('--ui-font', t.font);
    // Translucent variants used in lots of places
    const verifierRgb = hexToRgb(t.verifierBr);
    const refusedRgb  = hexToRgb(t.refusedBr);
    set('--verifier-rgb', verifierRgb);
    set('--refused-rgb', refusedRgb);
    // Body class for theme-conditional tweaks (extra glyphs, decorations)
    document.body.dataset.theme = tweaks.accent;
  }, [tweaks.accent]);

  const openBundle = (backend, collection) => {
    setBundleSelection({ backend, collection });
    setRoute('bundle');
  };

  const currentWitness = RULAKE.BACKENDS[0].collections[2].witness; // memories
  const currentBundle = 'lake-prod/memories';

  return (
    <div className="app">
      <Topbar envTab={envTab} setEnvTab={setEnvTab} onMenu={() => setMobileNav(true)} />
      <div className={'mobile-overlay' + (mobileNav ? ' open' : '')} style={{display: mobileNav ? 'block' : 'none', position:'fixed', inset:0, background:'rgba(0,0,0,0.5)', zIndex:50}} onClick={() => setMobileNav(false)}></div>
      <Sidebar className={mobileNav ? 'open' : ''} route={route} setRoute={(r) => { setRoute(r); setMobileNav(false); }} currentWitness={currentWitness} currentBundle={currentBundle} />
      <main className="main">
        {route === 'stats'      && <StatsScreen liveData={liveData} envTab={envTab} />}
        {route === 'playground' && <PlaygroundScreen />}
        {route === 'browse'     && <BrowseScreen onOpenBundle={openBundle} envTab={envTab} />}
        {route === 'bundle'     && <BundleScreen bundle={bundleSelection} />}
        {route === 'audit'      && <AuditScreen envTab={envTab} />}
        {route === 'connect'    && <ConnectScreen />}
        {route === 'appstore'   && <AppStoreScreen />}
      </main>
      <Statusbar stats={liveData} />
      <DebugConsole visible={tweaks.showConsole} onClose={() => setTweak('showConsole', false)} />
      <ToastHost />

      <WelcomeModal
        open={welcomeOpen}
        onClose={() => setWelcomeOpen(false)}
        onJump={(r) => setRoute(r)}
      />
      <HelpIndexModal
        open={helpIndexOpen}
        onClose={() => setHelpIndexOpen(false)}
        onTopic={(t) => { setHelpIndexOpen(false); setHelpTopic(t); }}
      />
      <HelpModal
        open={!!helpTopic}
        topicId={helpTopic}
        onClose={() => setHelpTopic(null)}
      />
      <HelpHotkey onOpenIndex={() => setHelpIndexOpen(true)} />

      <TweaksPanel>
        <TweakSection label="Aesthetic" />
        <TweakRadio label="Accent" value={tweaks.accent}
          options={['verifier-green', 'cool-teal', 'iris']}
          onChange={v => setTweak('accent', v)} />
        <TweakRadio label="Density" value={tweaks.density}
          options={['terminal', 'roomy']}
          onChange={v => setTweak('density', v)} />
        <TweakSection label="Display" />
        <TweakToggle label="Vector field" value={tweaks.showVectorField}
          onChange={v => setTweak('showVectorField', v)} />
        <TweakSlider label="Motion speed" min={0.2} max={2} step={0.1} unit="×"
          value={tweaks.motionSpeed} onChange={v => setTweak('motionSpeed', v)} />
        <TweakSection label="Debug" />
        <TweakToggle label="Console" value={tweaks.showConsole}
          onChange={v => setTweak('showConsole', v)} />
        <TweakSection label="Help" />
        <button className="btn" style={{width:'100%', marginTop:4}} onClick={() => setWelcomeOpen(true)}>
          Open welcome tour
        </button>
      </TweaksPanel>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById('root')).render(<App />);
