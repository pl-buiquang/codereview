import { BINDINGS } from "../lib/keyboard";

/**
 * The `?` keyboard-shortcuts overlay. Reuses the existing `.modal-backdrop` /
 * `.modal` pattern: backdrop click and the ✕ button call `onClose`; clicking the
 * panel body is swallowed. Escape is handled by the ReviewView dispatcher, not
 * here, so there is no window listener in this component.
 */
export function ShortcutHelp({ onClose }: { onClose: () => void }) {
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal shortcut-help" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>Keyboard shortcuts</h3>
          <button className="btn-icon" title="Close" onClick={onClose}>
            ✕
          </button>
        </div>
        <div className="shortcut-help-rows">
          {BINDINGS.map((b) => (
            <div className="shortcut-help-row" key={b.description}>
              <span className="shortcut-help-keys">
                {b.keys.map((k) => (
                  <kbd key={k}>{k}</kbd>
                ))}
              </span>
              <span className="shortcut-help-desc">{b.description}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
