// These modules register their APIs on window in dependency order.
// Static imports let Vite deliver the console in one request.
import './lib/data.js';
import './lib/store.js';
import './lib/wasm.js';
import './lib/searchOffload.js';
import './lib/embed.js';
import './components/tweaks-panel.jsx';
import './components/components.jsx';
import './components/modals.jsx';
import './components/toast.jsx';
import './components/help.jsx';
import './components/welcome.jsx';
import './components/screens.jsx';
import './components/debug.jsx';
import './components/app.jsx';
