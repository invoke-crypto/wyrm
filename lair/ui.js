const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

class Html {
  constructor(s) {
    this.s = s;
  }
  toString() {
    return this.s;
  }
}

function piece(v) {
  if (v == null || v === false) return '';
  if (v instanceof Html) return v.s;
  if (Array.isArray(v)) return v.map(piece).join('');
  return String(v).replace(/[&<>"']/g, (c) => ESC[c]);
}

export function html(strings, ...values) {
  let out = strings[0];
  for (let i = 0; i < values.length; i++) out += piece(values[i]) + strings[i + 1];
  return new Html(out);
}

export const raw = (s) => new Html(s);

export function toast(message, kind = 'info') {
  const region = document.getElementById('toasts');
  const el = document.createElement('div');
  el.className = `toast toast-${kind}`;
  el.setAttribute('role', kind === 'error' ? 'alert' : 'status');
  el.textContent = message;
  region.append(el);
  setTimeout(() => el.remove(), kind === 'error' ? 9000 : 5000);
}

// Opens a <dialog> built from `content`, resolves with the submitted FormData or null.
export function ask({ title, content, submit, danger = false }) {
  const dlg = document.createElement('dialog');
  dlg.className = 'ask';
  dlg.setAttribute('aria-labelledby', 'ask-title');
  dlg.innerHTML = String(html`
    <form method="dialog">
      <h2 id="ask-title">${title}</h2>
      ${content}
      <div class="ask-actions">
        <button type="button" class="btn btn-quiet" value="cancel">Cancel</button>
        <button type="submit" class="btn ${danger ? 'btn-danger' : 'btn-primary'}" value="ok">${submit}</button>
      </div>
    </form>`);
  document.body.append(dlg);
  return new Promise((resolve) => {
    const form = dlg.querySelector('form');
    let result = null;
    form.addEventListener('submit', (e) => {
      if (!form.reportValidity()) return e.preventDefault();
      result = new FormData(form);
    });
    dlg.querySelector('[value=cancel]').addEventListener('click', () => dlg.close());
    form.addEventListener('input', () => dlg.dispatchEvent(new Event('ask-input')));
    dlg.addEventListener('close', () => {
      dlg.remove();
      resolve(result);
    });
    dlg.showModal();
    dlg.querySelector('input, select, textarea, [type=submit]')?.focus();
  });
}
