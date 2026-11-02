import { useId, type ReactNode } from "react";

export function SettingsGroup({ title, description, action, children }: {
  title: string; description?: string; action?: ReactNode; children: ReactNode;
}) {
  return <section className="settings-group" aria-label={title}>
    <div className="settings-group-heading">
      <h3 className="settings-section-title">{title}</h3>
      {action}
    </div>
    {description && <p className="settings-help mb-2">{description}</p>}
    <div className="settings-card">{children}</div>
  </section>;
}

export function SettingsRow({ label, description, controlId, descriptionId, children }: {
  label: string; description?: string; controlId?: string; descriptionId?: string; children: ReactNode;
}) {
  return <div className="settings-row">
    <div className="settings-row-copy">
      {controlId ? <label htmlFor={controlId} className="settings-label">{label}</label> : <span className="settings-label">{label}</span>}
      {description && <p id={descriptionId} className="settings-help mt-0.5">{description}</p>}
    </div>
    <div className="settings-row-control">{children}</div>
  </div>;
}

export function SettingsToggle({ label, description, checked, disabled = false, onChange }: {
  label: string; description?: string; checked: boolean; disabled?: boolean; onChange: (checked: boolean) => void;
}) {
  const id = useId();
  return <SettingsRow label={label} description={description} controlId={id} descriptionId={`${id}-help`}>
    <button id={id} type="button" role="switch" aria-label={label} aria-describedby={description ? `${id}-help` : undefined} aria-checked={checked} disabled={disabled}
      className="settings-switch" onClick={() => onChange(!checked)}>
      <span aria-hidden="true" className="settings-switch-thumb" />
    </button>
  </SettingsRow>;
}
