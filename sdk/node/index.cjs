// CJS entry — re-exports the native binding (loaded via binding.js).
// ESM users land in index.mjs; CJS users land here. Both wrap errors
// thrown by the native side into a typed RuLakeError class with a
// `.code` discriminator (ADR-003 §6).

const native = require("./binding.cjs");

class RuLakeError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "RuLakeError";
    this.code = code;
  }
}

function rewrap(e) {
  if (!(e instanceof Error)) return e;
  // The native side stringifies as "RULAKE_*: <message>". Split once.
  const m = /^(RULAKE_[A-Z_]+):\s+(.*)$/s.exec(e.message);
  if (!m) return e;
  const wrapped = new RuLakeError(m[1], m[2]);
  wrapped.stack = e.stack;
  return wrapped;
}

// Wraps a method to rewrap thrown / rejected errors. Detects async
// vs sync at *call time* because napi-rs methods that return Promise<T>
// are still declared as ordinary functions (not `async fn` at the JS
// level), so `original.constructor.name` is "Function". The runtime
// check on the return value handles both.
function wrapMethod(fn) {
  return function (...args) {
    let out;
    try {
      out = fn.apply(this, args);
    } catch (e) {
      throw rewrap(e);
    }
    if (out && typeof out.then === "function") {
      return out.then(undefined, (e) => { throw rewrap(e); });
    }
    return out;
  };
}

// Walk every method on the native classes and wrap with rewrap so
// JS users see a consistent RuLakeError instead of plain Error.
function wrapClass(Cls) {
  const proto = Cls.prototype;
  for (const name of Object.getOwnPropertyNames(proto)) {
    if (name === "constructor") continue;
    const desc = Object.getOwnPropertyDescriptor(proto, name);
    if (!desc || typeof desc.value !== "function") continue;
    Object.defineProperty(proto, name, { ...desc, value: wrapMethod(desc.value) });
  }
}

wrapClass(native.RuLake);
wrapClass(native.LocalBackend);
wrapClass(native.FsBackend);
wrapClass(native.Bundle);

module.exports = {
  RuLake:        native.RuLake,
  LocalBackend:  native.LocalBackend,
  FsBackend:     native.FsBackend,
  Bundle:        native.Bundle,
  Consistency:   native.Consistency,
  RuLakeError,
};
