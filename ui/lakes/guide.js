const form = document.querySelector('#search');
const result = document.querySelector('#result');
const picker = document.querySelector('#snapshot');
const reset = document.querySelector('#reset');
const submit = form.querySelector('[type=submit]');
const sampleUrl = new URL('sample.ruvec1', document.baseURI).href;
let worker = null;
let snapshot = null;

function selectSnapshot(file) {
  worker?.terminate();
  worker = null;
  snapshot = file;
  submit.disabled = false;
  reset.hidden = !file;
  result.textContent = file ? file.name + ' selected. Enter a query of the same dimension.' : 'Sample ready · 8 vectors · 4 dimensions';
}

picker.addEventListener('change', () => selectSnapshot(picker.files[0] || null));
reset.addEventListener('click', () => {
  picker.value = '';
  selectSnapshot(null);
  form.elements.query.value = '1, 0, 0, 0';
});

form.hidden = false;
form.addEventListener('submit', event => {
  event.preventDefault();
  const parts = form.elements.query.value.split(',').map(s => s.trim());
  const query = Float32Array.from(parts, Number);
  if (parts.includes('') || query.length > 8192 || !query.every(Number.isFinite)) {
    result.textContent = 'Enter between 1 and 8192 finite numbers separated by commas.';
    return;
  }
  if (snapshot && snapshot.size > 16 * 1024 * 1024) {
    result.textContent = 'This file exceeds 16 MiB. Use the MCP server for larger snapshots.';
    return;
  }
  submit.disabled = true;
  result.textContent = 'Searching locally…';
  try {
    if (!worker) {
      worker = new Worker(new URL('./search.js', import.meta.url), { type: 'module' });
      worker.onmessage = ({ data }) => {
        result.textContent = data.error || ('ID ' + data.id + ' · squared L2 distance ' + data.score + ' · ' + data.rows + ' vectors searched');
        submit.disabled = false;
      };
      worker.onerror = () => {
        worker.terminate();
        worker = null;
        result.textContent = 'Local search could not start. Retry or use the MCP example below.';
        submit.disabled = false;
      };
    }
    worker.postMessage({ query, file: snapshot, sampleUrl });
  } catch (error) {
    result.textContent = error.message;
    submit.disabled = false;
  }
});

function markSource() {
  for (const link of document.querySelectorAll('.rail a')) {
    if (link.hash === location.hash) link.setAttribute('aria-current', 'location');
    else link.removeAttribute('aria-current');
  }
}
window.addEventListener('hashchange', markSource);
markSource();
