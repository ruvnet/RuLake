// ESM entry — re-exports from the CJS shim. The CJS file does the
// error-rewrapping once; both surfaces share that behavior. ESM users
// get tree-shakeable named imports.

import mod from "./index.cjs";

export const RuLake       = mod.RuLake;
export const LocalBackend = mod.LocalBackend;
export const FsBackend    = mod.FsBackend;
export const Bundle       = mod.Bundle;
export const Consistency  = mod.Consistency;
export const RuLakeError  = mod.RuLakeError;

export default mod;
