/* global React */
const { useState: useStT, useEffect: useEffT, useCallback: useCbT } = React;

// Toast bus — components dispatch via window.toast.push({...})
const _toastListeners = new Set();
let _toastId = 0;

window.toast = {
  push(toast) {
    const t = {
      id: ++_toastId,
      kind: 'ok', // ok | warn | err | info
      title: '',
      body: '',
      duration: 3200,
      ...toast,
    };
    _toastListeners.forEach(fn => fn({ type: 'add', toast: t }));
    if (t.duration > 0) setTimeout(() => {
      _toastListeners.forEach(fn => fn({ type: 'remove', id: t.id }));
    }, t.duration);
    return t.id;
  },
  ok(title, body, opts={}) { return this.push({ kind:'ok', title, body, ...opts }); },
  warn(title, body, opts={}) { return this.push({ kind:'warn', title, body, ...opts }); },
  err(title, body, opts={}) { return this.push({ kind:'err', title, body, ...opts }); },
  info(title, body, opts={}) { return this.push({ kind:'info', title, body, ...opts }); },
  dismiss(id) { _toastListeners.forEach(fn => fn({ type: 'remove', id })); },
};

function ToastHost() {
  const [toasts, setToasts] = useStT([]);
  useEffT(() => {
    const onEvt = (evt) => {
      if (evt.type === 'add') setToasts(t => [...t, evt.toast]);
      else if (evt.type === 'remove') setToasts(t => t.filter(x => x.id !== evt.id));
    };
    _toastListeners.add(onEvt);
    return () => _toastListeners.delete(onEvt);
  }, []);
  if (toasts.length === 0) return null;
  return (
    <div className="toast-host" role="region" aria-label="Notifications">
      {toasts.map(t => (
        <div key={t.id} className={'toast toast-' + t.kind}>
          <div className="toast-glyph">
            {t.kind === 'ok'   && <svg width="14" height="14" viewBox="0 0 14 14"><path d="M2 7l3 3 7-7" stroke="currentColor" strokeWidth="1.6" fill="none"/></svg>}
            {t.kind === 'warn' && <svg width="14" height="14" viewBox="0 0 14 14"><path d="M7 2l5.5 10H1.5L7 2z" stroke="currentColor" strokeWidth="1.4" fill="none"/><path d="M7 6v3" stroke="currentColor" strokeWidth="1.4"/><circle cx="7" cy="11" r="0.7" fill="currentColor"/></svg>}
            {t.kind === 'err'  && <svg width="14" height="14" viewBox="0 0 14 14"><circle cx="7" cy="7" r="5.5" stroke="currentColor" strokeWidth="1.4" fill="none"/><path d="M4.5 4.5l5 5M9.5 4.5l-5 5" stroke="currentColor" strokeWidth="1.4"/></svg>}
            {t.kind === 'info' && <svg width="14" height="14" viewBox="0 0 14 14"><circle cx="7" cy="7" r="5.5" stroke="currentColor" strokeWidth="1.4" fill="none"/><path d="M7 6v4M7 4v0.1" stroke="currentColor" strokeWidth="1.4"/></svg>}
          </div>
          <div className="toast-body">
            <div className="toast-title">{t.title}</div>
            {t.body && <div className="toast-sub">{t.body}</div>}
          </div>
          <button className="toast-x" onClick={() => window.toast.dismiss(t.id)} aria-label="dismiss">×</button>
        </div>
      ))}
    </div>
  );
}

window.RuToast = { ToastHost };
