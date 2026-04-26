/* Mock data + helpers shared across components */

const RULAKE = (() => {
  // Deterministic PRNG
  function mulberry32(a) {
    return function() {
      let t = a += 0x6D2B79F5;
      t = Math.imul(t ^ t >>> 15, t | 1);
      t ^= t + Math.imul(t ^ t >>> 7, t | 61);
      return ((t ^ t >>> 14) >>> 0) / 4294967296;
    };
  }

  const rand = mulberry32(0xc0ffee);

  // Generate witness hex
  function hex(n=64, seed=0) {
    const r = mulberry32(seed || 0xdea58c64);
    let s = '';
    const chars = '0123456789abcdef';
    for (let i = 0; i < n; i++) s += chars[Math.floor(r()*16)];
    return s;
  }

  function elide(h, head=8, tail=6) {
    if (!h || h.length < head+tail+2) return h;
    return h.slice(0, head) + '…' + h.slice(-tail);
  }

  const BACKENDS = [
    {
      id: 'lake-prod',
      kind: 'gcs',
      region: 'us-east-1',
      collections: [
        { id: 'docs.public',     dim: 1024, gen: 'Num(4128)',  entries: 481_223, witness: hex(64, 1), state: 'warm', hits: 24830, misses: 312, primes: 142, lastPrimeMs: 8.4 },
        { id: 'docs.internal',   dim: 1024, gen: 'Num(2210)',  entries: 92_410,  witness: hex(64, 2), state: 'warm', hits: 11920, misses: 802, primes:  88, lastPrimeMs: 12.1 },
        { id: 'memories',        dim: 768,  gen: 'Num(7741)',  entries: 1_204_998, witness: hex(64, 3), state: 'warm', hits: 88210, misses: 1410, primes: 412, lastPrimeMs: 6.2 },
        { id: 'tickets.support', dim: 512,  gen: 'Opaque(8a3)', entries: 38_220, witness: hex(64, 4), state: 'cold', hits: 0,    misses: 0,  primes: 0, lastPrimeMs: 0 },
      ],
    },
    {
      id: 'lake-eu',
      kind: 'fs',
      region: 'eu-west-2',
      collections: [
        { id: 'docs.public',     dim: 1024, gen: 'Num(4128)', entries: 481_223, witness: hex(64, 5), state: 'warm', hits: 9120, misses: 244, primes: 71, lastPrimeMs: 9.0 },
        { id: 'memories',        dim: 768,  gen: 'Num(7740)', entries: 1_204_220, witness: hex(64, 6), state: 'degraded', hits: 4012, misses: 320, primes: 22, lastPrimeMs: 18.2 },
      ],
    },
    {
      id: 'lake-edge',
      kind: 'ipfs',
      region: 'global',
      collections: [
        { id: 'changelog',       dim: 384,  gen: 'Opaque(7e2)', entries: 12_840, witness: hex(64, 7), state: 'cold', hits: 0, misses: 0, primes: 0, lastPrimeMs: 0 },
      ],
    },
  ];

  // Audit lines — cycle of recent events
  const AUDIT = [
    { ts: '13:41:08.812', principal: 'agent-7f',   tool: 'rulake_query',          intent: 'search', target: 'lake-prod/memories',     outcome: 'ok',       code: 'OK',                       k: 10, ms: 11 },
    { ts: '13:41:08.731', principal: 'agent-7f',   tool: 'rulake_query',          intent: 'search', target: 'lake-prod/docs.public',  outcome: 'ok',       code: 'OK',                       k: 5,  ms: 6  },
    { ts: '13:41:07.402', principal: 'jules@ruv',  tool: 'rulake_publish_bundle', intent: '-',      target: 'lake-prod/docs.internal',outcome: 'ok',       code: 'OK',                       k: '-', ms: 244 },
    { ts: '13:41:06.991', principal: 'agent-7f',   tool: 'rulake_query',          intent: 'search', target: 'lake-eu/memories',       outcome: 'degraded', code: 'STALE_BUNDLE_FALLBACK',     k: 10, ms: 28 },
    { ts: '13:41:06.044', principal: 'agent-q2',   tool: 'rulake_query',          intent: 'search', target: 'lake-prod/docs.public',  outcome: 'refused',  code: 'WITNESS_MISMATCH_REFUSED',  k: 10, ms: 4  },
    { ts: '13:41:05.118', principal: 'agent-7f',   tool: 'rulake_query',          intent: 'search', target: 'lake-prod/memories',     outcome: 'ok',       code: 'OK',                       k: 10, ms: 9  },
    { ts: '13:41:04.840', principal: 'jules@ruv',  tool: 'rulake_warm_from_dir',  intent: '-',      target: 'lake-prod/tickets.support', outcome: 'ok',    code: 'OK',                       k: '-', ms: 1820 },
    { ts: '13:41:04.011', principal: 'agent-q2',   tool: 'rulake_query',          intent: 'search', target: 'lake-edge/changelog',    outcome: 'refused',  code: 'POLICY_DENIED',             k: 5,  ms: 2  },
    { ts: '13:41:03.512', principal: 'agent-7f',   tool: 'rulake_query',          intent: 'search', target: 'lake-prod/memories',     outcome: 'ok',       code: 'OK',                       k: 20, ms: 14 },
    { ts: '13:41:02.901', principal: 'agent-7f',   tool: 'rulake_query',          intent: 'search', target: 'lake-prod/docs.public',  outcome: 'ok',       code: 'OK',                       k: 10, ms: 8  },
    { ts: '13:41:02.402', principal: 'agent-q2',   tool: 'rulake_query',          intent: 'search', target: 'lake-eu/memories',       outcome: 'degraded', code: 'STALE_BUNDLE_FALLBACK',     k: 10, ms: 31 },
    { ts: '13:41:01.118', principal: 'agent-7f',   tool: 'rulake_query',          intent: 'search', target: 'lake-prod/memories',     outcome: 'ok',       code: 'OK',                       k: 10, ms: 7  },
  ];

  // Sample query result (search "Q4 onboarding metrics — drop after step 3")
  const SAMPLE_QUERY_RESPONSE = {
    request: {
      intent: 'search',
      target: { collection: 'memories' },
      search: { vector: '[1024 floats]', k: 10 },
      risk: 'medium',
      budget: { max_latency_ms: 100, max_results: 20 },
    },
    data: [
      { id: 'mem.81f3c2', score: 0.9214, snippet: 'Onboarding funnel — step 3 drop is 41% on cohort 2026-04. Hypothesis: passkey copy ambiguity.', source: 'memories', ts: '2026-04-22' },
      { id: 'mem.7e21a0', score: 0.8842, snippet: 'Step 3 friction analysis: passkey vs magic link split test. n=12,400. Magic link converts +18%.', source: 'memories', ts: '2026-04-19' },
      { id: 'mem.55cc14', score: 0.8511, snippet: 'Q4 onboarding plan: collapse step 3+4 into single screen. Owners: rin, jules.',                  source: 'memories', ts: '2026-04-04' },
      { id: 'mem.2afb88', score: 0.8419, snippet: 'Cohort 2026-Q1: post-onboarding D7 retention is 62% (target 70). Step-3 abandons re-enter at D2.', source: 'memories', ts: '2026-04-01' },
      { id: 'mem.0e3e92', score: 0.8002, snippet: 'Decision: ship the magic-link variant to 50% on 2026-05-04. Rollback gate at -3% activation.',  source: 'memories', ts: '2026-03-28' },
      { id: 'mem.41d720', score: 0.7811, snippet: 'Step 3 redesign sketch (paper). Move passkey behind "More options" disclosure.',                  source: 'memories', ts: '2026-03-20' },
      { id: 'mem.b80d44', score: 0.7588, snippet: 'Activation north-star moved from D1 to D7 to capture genuine onboarding completion.',             source: 'memories', ts: '2026-03-12' },
      { id: 'mem.92ee0c', score: 0.7402, snippet: 'PostHog funnel: step 3 has 1.8s median dwell, 4.4s p90. Cognitive load suspect.',                   source: 'memories', ts: '2026-03-08' },
      { id: 'mem.a13c50', score: 0.7180, snippet: 'Q3 retro: onboarding owners under-resourced. Allocate 0.5 FTE through Q4.',                          source: 'memories', ts: '2026-02-21' },
      { id: 'mem.6b71f8', score: 0.7044, snippet: 'User interview log: "I didnt know what passkey meant. I just closed the tab."',                     source: 'memories', ts: '2026-02-18' },
    ],
    provenance: {
      witness: hex(64, 3),
      bundle: 'lake-prod/memories',
      generation: 'Num(7741)',
      issued_at: '2026-04-26T13:41:08.812Z',
    },
    trust_level: 'verified',
    decision: {
      chosen_action: 'serve_from_cache',
      reason_code: 'CACHE_HIT_FRESH',
      backends_used: [
        { backend: 'lake-prod', collection: 'memories', latency_ms: 9.4, hits: 10, witness: hex(64, 3) },
      ],
      refusals: [
        { backend: 'lake-eu', collection: 'memories', code: 'STALE_BUNDLE_GUARD', reason: 'Cached witness is 1 generation behind freshness budget (1500ms)' },
      ],
      degraded_advice: { hints: [] },
      budget_used_ms: 11.2,
      budget_max_ms: 100,
    },
  };

  const REFUSAL_BREAKDOWN = [
    { code: 'WITNESS_MISMATCH_REFUSED',   count: 14 },
    { code: 'STALE_BUNDLE_GUARD',          count: 22 },
    { code: 'STALE_BUNDLE_FALLBACK',       count: 47 },
    { code: 'POLICY_DENIED',               count: 6 },
    { code: 'BUDGET_EXCEEDED',             count: 3 },
  ];

  return { hex, elide, mulberry32, BACKENDS, AUDIT, SAMPLE_QUERY_RESPONSE, REFUSAL_BREAKDOWN };
})();

window.RULAKE = RULAKE;
