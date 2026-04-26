/* @ts-self-types="./rulake_wasm.d.ts" */
import * as wasm from "./rulake_wasm_bg.wasm";
import { __wbg_set_wasm } from "./rulake_wasm_bg.js";

__wbg_set_wasm(wasm);

export {
    buildInfo, computeWitness, formatVersion, searchBruteForceL2, verifyBundleJson
} from "./rulake_wasm_bg.js";
