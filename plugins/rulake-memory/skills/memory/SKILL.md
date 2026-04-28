---
name: memory
description: Use ruLake as the agent's working memory. Auto-invoke /rulake-memory:memory-recall when the agent needs to look up something it might already know; auto-invoke /rulake-memory:memory-remember after the agent learns or decides something worth pinning. Closes the self-learning loop without requiring the agent author to wire it explicitly.
---

# Use ruLake as Working Memory

This skill turns ruLake into the agent's working memory transparently. Without an explicit slash command, the agent should call `memory-recall` before answering questions about facts it might already know, and call `memory-remember` after learning new facts or making decisions.

## When to recall

Before answering, ask:
- "Did the user already tell me this?" → `/rulake-memory:memory-recall`
- "Did I already decide this?" → `/rulake-memory:memory-recall`
- "Did I already do this lookup?" → `/rulake-memory:memory-recall` (cached tool results)
- "What's relevant context from past sessions?" → `/rulake-memory:memory-recall` with the topic

If `memory-recall` returns a hit with `decision_trace.witness.match: true`, use the cached answer. Skip the redundant work. Cite the witness in the response so the user knows it came from memory.

## When to remember

After completing work, ask:
- "Did I make a decision the user might re-ask about?" → `/rulake-memory:memory-remember`
- "Did the user share a preference / fact / constraint?" → `/rulake-memory:memory-remember key=user-prefs/...`
- "Did I run an expensive tool call whose result is reusable?" → `/rulake-memory:memory-remember key=tool-results/<call-hash>`
- "Did I learn something general about this domain?" → `/rulake-memory:memory-remember memory_class=semantic`

Pick the `memory_class` honestly:
- **`working`** — current task scratchpad (TTL-able)
- **`episodic`** — "this happened at time T in session S"
- **`semantic`** — general facts the agent should know forever
- **`procedural`** — "how to do X" — recipes / patterns

## When to refuse to remember

Don't pin things that change frequently — the witness will refuse on next recall and the cache churn defeats the purpose. Use `Eventual{ttl_ms}` consistency or skip remembering for:
- Live prices / market data
- Anything timestamped now()
- User questions that have user-specific context (remember the answer, not the question)

## Self-learning loop

Every recall feeds `/memory-status`. Every refusal feeds `/memory-tune`. Every drift feeds `/memory-replay`. The agent doesn't need to invoke those manually — the operator can wire them as `/loop`-driven workers, but the recall/remember pair is enough to make ruLake function as the agent's memory.

The whole point: memory that gets faster the more it's used, and refuses to make things up when it can't be sure.
