/* global React */
const { useState: useStW, useEffect: useEffW, useRef: useRefW } = React;

// ─── Intro slideshow — 5-frame animated explainer for step 1 ─────────────
function IntroSlideshow() {
  const [idx, setIdx] = useStW(0);
  const [paused, setPaused] = useStW(false);
  const timerRef = useRefW(null);

  const slides = [
    {
      tag: 'the problem',
      head: 'Your agent needs to remember.',
      body: <>Past chats. Documents you've shown it. Facts it has learned. Without memory, every conversation starts from zero.</>,
      art: (
        <svg viewBox="0 0 360 130" width="100%" height="130">
          <defs>
            <linearGradient id="ig-glow" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="#2d8c5d" stopOpacity="0.3"/>
              <stop offset="100%" stopColor="#2d8c5d" stopOpacity="0"/>
            </linearGradient>
          </defs>
          <g className="intro-anim-fade">
            <rect x="80" y="32" width="200" height="64" rx="2" fill="none" stroke="#7a8190" strokeWidth="1.2"/>
            <text x="180" y="56" textAnchor="middle" fill="#d6dae0" fontSize="13" fontFamily="Inter" fontWeight="500">YOUR AGENT</text>
            <text x="180" y="74" textAnchor="middle" fill="#7a8190" fontSize="10" fontFamily="JetBrains Mono">"what did we discuss yesterday?"</text>
            <text x="180" y="88" textAnchor="middle" fill="#5a6172" fontSize="9" fontFamily="JetBrains Mono">↓ no memory</text>
          </g>
          <g className="intro-anim-shake">
            <text x="180" y="118" textAnchor="middle" fill="#e89548" fontSize="11" fontFamily="JetBrains Mono">⚠ I don't know.</text>
          </g>
        </svg>
      ),
    },
    {
      tag: 'how memory works',
      head: 'Memory means finding similar things — fast.',
      body: <>Every memory is stored as an <strong>embedding</strong> — a numeric fingerprint of meaning. "Remembering" is finding the embeddings closest to what the agent is thinking about right now.</>,
      art: (
        <svg viewBox="0 0 360 130" width="100%" height="130">
          <g className="intro-anim-points">
            {[
              [60,30,'past chat'],[110,52,'doc'],[150,28,'fact'],[200,60,''],[240,40,''],[290,52,''],
              [80,90,''],[140,100,''],[200,82,''],[260,98,''],[310,80,''],[40,72,''],
            ].map(([x,y,lbl],i) => (
              <g key={i}>
                <circle cx={x} cy={y} r="3" fill="#4a9eb5" opacity="0.7"/>
                {lbl && <text x={x+6} y={y+3} fill="#7a8190" fontSize="8.5" fontFamily="JetBrains Mono">{lbl}</text>}
              </g>
            ))}
          </g>
          <g className="intro-anim-query">
            <circle cx="180" cy="65" r="5" fill="#2d8c5d"/>
            <circle cx="180" cy="65" r="14" fill="none" stroke="#2d8c5d" strokeWidth="0.8" opacity="0.8">
              <animate attributeName="r" from="5" to="40" dur="1.6s" repeatCount="indefinite"/>
              <animate attributeName="opacity" from="0.8" to="0" dur="1.6s" repeatCount="indefinite"/>
            </circle>
            <text x="190" y="68" fill="#2d8c5d" fontSize="10" fontFamily="JetBrains Mono" fontWeight="600">query</text>
          </g>
          <text x="180" y="120" textAnchor="middle" fill="#7a8190" fontSize="9.5" fontFamily="JetBrains Mono">find the closest dots → top-K matches</text>
        </svg>
      ),
    },
    {
      tag: 'the usual options',
      head: 'Three current options. All hurt.',
      body: <>You can run a <strong>vector database</strong> (slow to set up, your data has to move). Use a <strong>warehouse</strong> (every query bills the cluster). Or use a <strong>local library</strong> (every agent runs its own copy, nothing's shared, nothing's verifiable).</>,
      art: (
        <svg viewBox="0 0 360 130" width="100%" height="130">
          <g className="intro-anim-stagger">
            <g transform="translate(20,30)">
              <rect width="100" height="70" fill="none" stroke="#7a8190" strokeWidth="0.8"/>
              <text x="50" y="20" textAnchor="middle" fill="#d6dae0" fontSize="11" fontFamily="Inter" fontWeight="500">Vector DB</text>
              <text x="50" y="38" textAnchor="middle" fill="#7a8190" fontSize="8.5" fontFamily="JetBrains Mono">Pinecone</text>
              <text x="50" y="50" textAnchor="middle" fill="#7a8190" fontSize="8.5" fontFamily="JetBrains Mono">Weaviate</text>
              <text x="50" y="64" textAnchor="middle" fill="#e89548" fontSize="9" fontFamily="JetBrains Mono">+ new service</text>
            </g>
            <g transform="translate(130,30)">
              <rect width="100" height="70" fill="none" stroke="#7a8190" strokeWidth="0.8"/>
              <text x="50" y="20" textAnchor="middle" fill="#d6dae0" fontSize="11" fontFamily="Inter" fontWeight="500">Warehouse</text>
              <text x="50" y="38" textAnchor="middle" fill="#7a8190" fontSize="8.5" fontFamily="JetBrains Mono">BigQuery</text>
              <text x="50" y="50" textAnchor="middle" fill="#7a8190" fontSize="8.5" fontFamily="JetBrains Mono">Snowflake</text>
              <text x="50" y="64" textAnchor="middle" fill="#e89548" fontSize="9" fontFamily="JetBrains Mono">+ pay per query</text>
            </g>
            <g transform="translate(240,30)">
              <rect width="100" height="70" fill="none" stroke="#7a8190" strokeWidth="0.8"/>
              <text x="50" y="20" textAnchor="middle" fill="#d6dae0" fontSize="11" fontFamily="Inter" fontWeight="500">Library</text>
              <text x="50" y="38" textAnchor="middle" fill="#7a8190" fontSize="8.5" fontFamily="JetBrains Mono">FAISS</text>
              <text x="50" y="50" textAnchor="middle" fill="#7a8190" fontSize="8.5" fontFamily="JetBrains Mono">RaBitQ</text>
              <text x="50" y="64" textAnchor="middle" fill="#e89548" fontSize="9" fontFamily="JetBrains Mono">+ no sharing</text>
            </g>
          </g>
          <text x="180" y="120" textAnchor="middle" fill="#7a8190" fontSize="9.5" fontFamily="JetBrains Mono">none of them are right for AI agents</text>
        </svg>
      ),
    },
    {
      tag: 'the missing middle',
      head: 'ruLake is the layer in between.',
      body: <>Your data <strong>stays where it lives</strong>. ruLake keeps a compressed copy in RAM, returns matches in <strong>~1 ms</strong>, and refreshes from the source when it changes.</>,
      art: (
        <svg viewBox="0 0 360 130" width="100%" height="130">
          <defs>
            <linearGradient id="ig-arr" x1="0" y1="0" x2="1" y2="0">
              <stop offset="0%" stopColor="#2d8c5d"/><stop offset="100%" stopColor="#4a9eb5"/>
            </linearGradient>
          </defs>
          <g transform="translate(8,38)" className="intro-anim-pop" style={{animationDelay:'0ms'}}>
            <rect width="76" height="44" fill="none" stroke="#7a8190" strokeWidth="1"/>
            <text x="38" y="20" textAnchor="middle" fill="#d6dae0" fontSize="10" fontFamily="JetBrains Mono">AGENT</text>
            <text x="38" y="34" textAnchor="middle" fill="#7a8190" fontSize="8" fontFamily="Inter">Claude · Cursor</text>
          </g>
          <g className="intro-anim-flow">
            <line x1="86" y1="60" x2="142" y2="60" stroke="url(#ig-arr)" strokeWidth="1.4"/>
            <polygon points="142,60 138,57 138,63" fill="#4a9eb5"/>
            <text x="114" y="54" textAnchor="middle" fill="#d6dae0" fontSize="9" fontFamily="JetBrains Mono">ask</text>
          </g>
          <g transform="translate(144,28)" className="intro-anim-pop" style={{animationDelay:'200ms'}}>
            <rect width="92" height="64" fill="none" stroke="url(#ig-arr)" strokeWidth="1.4"/>
            <text x="46" y="22" textAnchor="middle" fill="#2d8c5d" fontSize="13" fontWeight="600" fontFamily="JetBrains Mono">ruLake</text>
            <text x="46" y="38" textAnchor="middle" fill="#7a8190" fontSize="8.5" fontFamily="Inter">~1 ms · in RAM</text>
            <text x="46" y="52" textAnchor="middle" fill="#7a8190" fontSize="8.5" fontFamily="Inter">32× compressed</text>
          </g>
          <g className="intro-anim-flow" style={{animationDelay:'400ms'}}>
            <line x1="238" y1="60" x2="294" y2="60" stroke="#4a9eb5" strokeWidth="1.4" strokeDasharray="3 2"/>
            <polygon points="294,60 290,57 290,63" fill="#4a9eb5"/>
            <text x="266" y="54" textAnchor="middle" fill="#7a8190" fontSize="9" fontFamily="JetBrains Mono">refresh</text>
          </g>
          <g transform="translate(296,18)" className="intro-anim-pop" style={{animationDelay:'400ms'}} fill="#7a8190" fontFamily="JetBrains Mono" fontSize="8.5">
            <rect width="56" height="14" fill="none" stroke="#7a8190" strokeWidth="0.6"/>
            <text x="28" y="10" textAnchor="middle" fill="#d6dae0">S3 / GCS</text>
            <rect y="20" width="56" height="14" fill="none" stroke="#7a8190" strokeWidth="0.6"/>
            <text x="28" y="30" textAnchor="middle" fill="#d6dae0">BigQuery</text>
            <rect y="40" width="56" height="14" fill="none" stroke="#7a8190" strokeWidth="0.6"/>
            <text x="28" y="50" textAnchor="middle" fill="#d6dae0">Parquet</text>
            <rect y="60" width="56" height="14" fill="none" stroke="#7a8190" strokeWidth="0.6"/>
            <text x="28" y="70" textAnchor="middle" fill="#d6dae0">Iceberg</text>
            <text x="28" y="86" textAnchor="middle" fill="#5a6172" fontSize="7.5" fontFamily="Inter">your data, where it is</text>
          </g>
        </svg>
      ),
    },
    {
      tag: 'why agents trust it',
      head: 'Every answer carries a fingerprint.',
      body: <>A small SHAKE-256 hash of the data the answer came from. Two agents on two machines querying the same data get the <strong>same fingerprint</strong> — and the same answer, byte for byte. If the local copy is stale and the source is unreachable, ruLake <strong>refuses</strong> instead of guessing.</>,
      art: (
        <svg viewBox="0 0 360 130" width="100%" height="130">
          <g className="intro-anim-pop" style={{animationDelay:'0ms'}}>
            <rect x="40" y="22" width="280" height="36" rx="2" fill="none" stroke="#2d8c5d" strokeWidth="1.2"/>
            <text x="55" y="44" fill="#2d8c5d" fontSize="10.5" fontFamily="JetBrains Mono">witness:</text>
            <text x="118" y="44" fill="#d6dae0" fontSize="11" fontFamily="JetBrains Mono">b071c77248f7febafc0…</text>
            <text x="290" y="44" fill="#2d8c5d" fontSize="10" fontFamily="JetBrains Mono" fontWeight="600">● MATCH</text>
          </g>
          <g className="intro-anim-pop" style={{animationDelay:'200ms'}}>
            <rect x="40" y="68" width="280" height="36" rx="2" fill="none" stroke="#e89548" strokeWidth="1.2"/>
            <text x="55" y="90" fill="#e89548" fontSize="10.5" fontFamily="JetBrains Mono">stale + unreachable:</text>
            <text x="180" y="90" fill="#d6dae0" fontSize="11" fontFamily="JetBrains Mono">honest refusal</text>
            <text x="290" y="90" fill="#e89548" fontSize="10" fontFamily="JetBrains Mono" fontWeight="600">● NO ANSWER</text>
          </g>
          <text x="180" y="124" textAnchor="middle" fill="#7a8190" fontSize="9.5" fontFamily="JetBrains Mono">verifiable across hosts · honest by design</text>
        </svg>
      ),
    },
  ];

  // Auto-advance
  useEffW(() => {
    if (paused) return;
    timerRef.current = setTimeout(() => setIdx(i => (i + 1) % slides.length), 5500);
    return () => clearTimeout(timerRef.current);
  }, [idx, paused, slides.length]);

  const slide = slides[idx];

  return (
    <div className="intro-show">
      <div className="intro-show-frame">
        <div className="intro-show-tag">
          <span className="intro-show-tag-dot"></span>
          <span>{slide.tag}</span>
          <span className="intro-show-counter">{idx + 1} / {slides.length}</span>
        </div>
        <div key={idx} className="intro-show-art">{slide.art}</div>
        <div key={'t'+idx} className="intro-show-copy">
          <h3 className="intro-show-head">{slide.head}</h3>
          <p className="intro-show-body">{slide.body}</p>
        </div>
      </div>
      <div className="intro-show-controls">
        <button
          className="intro-show-arrow"
          onClick={() => { setPaused(true); setIdx(i => (i - 1 + slides.length) % slides.length); }}
          aria-label="Previous slide"
          title="Previous"
        >‹</button>
        <div className="intro-show-dots">
          {slides.map((_, i) => (
            <button
              key={i}
              className={'intro-show-dot' + (i === idx ? ' active' : '')}
              onClick={() => { setPaused(true); setIdx(i); }}
              aria-label={`Go to slide ${i + 1}`}
            />
          ))}
        </div>
        <button
          className="intro-show-arrow"
          onClick={() => { setPaused(true); setIdx(i => (i + 1) % slides.length); }}
          aria-label="Next slide"
          title="Next"
        >›</button>
        <button className="intro-show-pause" onClick={() => setPaused(p => !p)}>
          {paused ? '▶ play' : '⏸ pause'}
        </button>
      </div>
    </div>
  );
}

// Welcome / first-run guide — 4-step modal walkthrough
function WelcomeModal({ open, onClose, onJump }) {
  const [step, setStep] = useStW(0);
  const [endpoint, setEndpoint] = useStW('https://rulake.ruv.net/mcp');
  const [authMode, setAuthMode] = useStW('jwt');
  const [accent, setAccent] = useStW('verifier-green');
  useEffW(() => { if (open) setStep(0); }, [open]);

  // Apply accent instantly when picked, so step 3 actually previews the theme
  const applyAccent = (id) => {
    setAccent(id);
    window.parent.postMessage({type:'__edit_mode_set_keys', edits:{ accent: id }}, '*');
  };

  if (!open) return null;

  const steps = [
    {
      title: 'Memory for AI agents — without a vector database',
      kicker: 'v0.3.1 · what ruLake actually is',
      body: (
        <div className="welcome-step">
          <IntroSlideshow />
          <div className="welcome-hint" style={{marginTop:14, textAlign:'center'}}>
            This console is the operator's lens — watch queries flow, inspect witnesses, audit refusals, and tune cache across every lake you've connected.
          </div>
        </div>
      ),
    },
    {
      title: 'Connect your MCP endpoint',
      kicker: 'step 1 of 3',
      body: (
        <div className="welcome-step">
          <p className="welcome-lead">
            Point the console at any rulake-mcp server. Don't have one yet?
            Skip and explore the demo data — every screen works offline.
          </p>
          <div className="welcome-form">
            <label className="welcome-field">
              <span className="welcome-field-l">Endpoint URL</span>
              <input className="input" value={endpoint} onChange={e=>setEndpoint(e.target.value)} />
            </label>
            <label className="welcome-field">
              <span className="welcome-field-l">Auth</span>
              <div className="seg">
                {['none','bearer','jwt','mtls'].map(m => (
                  <div key={m} className={'seg-item' + (authMode===m?' active':'')} onClick={()=>setAuthMode(m)}>{m}</div>
                ))}
              </div>
            </label>
            {authMode === 'mtls' && (
              <div className="welcome-warn">
                ⚠ mTLS needs a non-browser client — the TLS handshake picks the cert
                before any JS runs. Use rulake-cli or a Node client.
              </div>
            )}
            <div className="welcome-hint">
              Saves to local IndexedDB only · never sent off-device
            </div>
          </div>
        </div>
      ),
    },
    {
      title: 'Pick an environment',
      kicker: 'step 2 of 3',
      body: (
        <div className="welcome-step">
          <p className="welcome-lead">
            Use the topbar tabs to switch between <strong>lake-prod</strong>,
            <strong> lake-eu</strong>, and <strong>lake-edge</strong>.
            Every screen — Stats, Backends, Audit — re-scopes to the selected lake.
          </p>
          <div className="welcome-envs">
            <div className="welcome-env">
              <div className="welcome-env-dot" style={{background:'#2d8c5d'}}></div>
              <div>
                <div className="welcome-env-name">lake-prod</div>
                <div className="welcome-env-meta">us-west · primary · 100% load</div>
              </div>
            </div>
            <div className="welcome-env">
              <div className="welcome-env-dot" style={{background:'#4a9eb5'}}></div>
              <div>
                <div className="welcome-env-name">lake-eu</div>
                <div className="welcome-env-meta">eu-west · replica · 42% load</div>
              </div>
            </div>
            <div className="welcome-env">
              <div className="welcome-env-dot" style={{background:'#e89548'}}></div>
              <div>
                <div className="welcome-env-name">lake-edge</div>
                <div className="welcome-env-meta">pop-* · degraded · 8% load</div>
              </div>
            </div>
          </div>
          <div className="welcome-hint">
            Tip — switch tabs while watching the Stats charts to see per-lake throughput, hit-rate, and refusal patterns.
          </div>
        </div>
      ),
    },
    {
      title: 'Choose your aesthetic',
      kicker: 'step 3 of 3',
      body: (
        <div className="welcome-step">
          <p className="welcome-lead">
            Toggle <strong>Tweaks</strong> from the toolbar to change accent, density,
            motion, and toggle a debug console. You can change all of these later.
          </p>
          <div className="welcome-accents">
            {[
              { id: 'verifier-green', name: 'Verifier green', swatches: ['#0a0e14', '#11161e', '#2d8c5d', '#e89548', '#4a9eb5'], sub: 'deep slate · forest verifier · amber refusals' },
              { id: 'cool-teal',      name: 'Cool teal',      swatches: ['#0b1316', '#121d22', '#2dabab', '#d29a6a', '#5fb6c4'], sub: 'midnight · low-stim teal · muted refusals' },
              { id: 'iris',           name: 'Iris',           swatches: ['#0d0c14', '#15131e', '#7a78d6', '#d678b3', '#9c7ad6'], sub: 'editorial · iris accent · magenta refusals' },
            ].map(a => (
              <div key={a.id} className={'welcome-accent' + (accent===a.id?' active':'')} onClick={()=>applyAccent(a.id)}>
                <div className="welcome-accent-swatches">
                  {a.swatches.map((c,i) => (
                    <div key={i} className="welcome-accent-chip" style={{background:c}}></div>
                  ))}
                </div>
                <div>
                  <div className="welcome-accent-name">{a.name}</div>
                  <div className="welcome-accent-sub">{a.sub}</div>
                </div>
                {accent === a.id && <div className="welcome-accent-check">✓</div>}
              </div>
            ))}
          </div>
          <div className="welcome-hint">
            Pick one — the console behind the modal updates live. Background, surfaces, accent, and refusal tones all swap together.
          </div>
        </div>
      ),
    },
    {
      title: "You're set — explore the console",
      kicker: 'where to go from here',
      body: (
        <div className="welcome-step">
          <p className="welcome-lead">
            Each screen does one thing. Click a card below to jump in, or use the sidebar.
          </p>
          <div className="welcome-grid">
            <div className="welcome-card" onClick={()=>{onJump('stats'); onClose();}}>
              <div className="welcome-card-h">▦ Stats</div>
              <div className="welcome-card-b">Live throughput, hit-rate, latency. Per-lake rollup.</div>
            </div>
            <div className="welcome-card" onClick={()=>{onJump('playground'); onClose();}}>
              <div className="welcome-card-h">▶ Playground</div>
              <div className="welcome-card-b">Send <code>rulake_query</code>, see witness, save queries.</div>
            </div>
            <div className="welcome-card" onClick={()=>{onJump('browse'); onClose();}}>
              <div className="welcome-card-h">◫ Backends</div>
              <div className="welcome-card-b">All collections, cache pressure, federation graph.</div>
            </div>
            <div className="welcome-card" onClick={()=>{onJump('bundle'); onClose();}}>
              <div className="welcome-card-h">◉ Bundle</div>
              <div className="welcome-card-b">Witness comparator. Pin gen-7741 for later diff.</div>
            </div>
            <div className="welcome-card" onClick={()=>{onJump('audit'); onClose();}}>
              <div className="welcome-card-h">≡ Audit</div>
              <div className="welcome-card-b">JSONL tail of every request. Filterable.</div>
            </div>
            <div className="welcome-card" onClick={()=>{onJump('connect'); onClose();}}>
              <div className="welcome-card-h">⌁ Connect</div>
              <div className="welcome-card-b">Manage saved endpoints &amp; capability matrix.</div>
            </div>
          </div>
        </div>
      ),
    },
  ];

  const cur = steps[step];
  const isLast = step === steps.length - 1;
  const isFirst = step === 0;

  const finish = async () => {
    if (window.RuStore) {
      await window.RuStore.setKV('welcome_seen', true);
      // Persist initial connection if user kept defaults
      if (endpoint && authMode !== 'mtls') {
        await window.RuStore.put('connections', { label: 'first-run', endpoint, mode: authMode, token: '' });
      }
      // Persist accent
      window.parent.postMessage({type:'__edit_mode_set_keys', edits:{ accent }}, '*');
    }
    onClose();
  };

  return (
    <div className="modal-backdrop welcome-backdrop" onClick={(e) => { if (e.target.classList.contains('welcome-backdrop')) onClose(); }}>
      <div className="modal welcome-modal">
        <div className="welcome-rail">
          {steps.map((s, i) => (
            <div key={i} className={'welcome-rail-step' + (i===step?' active':'') + (i<step?' done':'')}>
              <div className="welcome-rail-dot">{i < step ? '✓' : i + 1}</div>
              <div className="welcome-rail-label">{s.title.split(' ').slice(0,3).join(' ')}</div>
            </div>
          ))}
        </div>
        <div className="welcome-pane">
          <div className="modal-head">
            <div>
              <div className="welcome-kicker">{cur.kicker}</div>
              <div className="modal-title" style={{fontSize:18, marginTop:2}}>{cur.title}</div>
            </div>
            <button className="modal-x" onClick={onClose}>×</button>
          </div>
          <div className="modal-body welcome-body">{cur.body}</div>
          <div className="modal-foot welcome-foot">
            <button className="btn" onClick={onClose}>Skip tour</button>
            <a className="welcome-gh" href="https://github.com/ruvnet/RuLake" target="_blank" rel="noopener">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor" style={{marginRight:6, verticalAlign:'middle'}}><path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.1.79-.25.79-.55 0-.27-.01-1-.02-1.96-3.2.69-3.87-1.54-3.87-1.54-.52-1.33-1.28-1.68-1.28-1.68-1.05-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.24-1.27-5.24-5.66 0-1.25.45-2.27 1.18-3.07-.12-.29-.51-1.46.11-3.04 0 0 .96-.31 3.15 1.17.91-.25 1.89-.38 2.86-.38.97 0 1.95.13 2.86.38 2.18-1.48 3.14-1.17 3.14-1.17.62 1.58.23 2.75.11 3.04.74.8 1.18 1.82 1.18 3.07 0 4.4-2.69 5.37-5.25 5.65.41.35.78 1.05.78 2.12 0 1.53-.01 2.76-.01 3.13 0 .3.21.66.8.55C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z"/></svg>
              ruvnet/RuLake
            </a>
            <div style={{flex:1}}></div>
            {!isFirst && <button className="btn" onClick={()=>setStep(s=>s-1)}>← Back</button>}
            {!isLast && <button className="btn btn-primary" onClick={()=>setStep(s=>s+1)}>Next →</button>}
            {isLast && <button className="btn btn-primary" onClick={finish}>Enter console</button>}
          </div>
        </div>
      </div>
    </div>
  );
}

window.RuWelcome = { WelcomeModal };
