import { DEFAULT_APPEARANCE, modeColors, type Appearance } from "../appearance";
import { SettingsGroup, SettingsRow } from "./SettingsRows";

export function AppearanceSettings({ value, mode, onChange }: { value: Appearance; mode: "light" | "dark"; onChange: (value: Appearance) => void }) {
  const set = <K extends keyof Appearance>(key: K, entry: Appearance[K]) => onChange({ ...value, [key]: entry });
  const colors = modeColors(value, mode);
  const backgroundKey = mode === "dark" ? "darkBackground" : "lightBackground";
  const foregroundKey = mode === "dark" ? "darkForeground" : "lightForeground";
  return <>
    <SettingsGroup title="Visual style" action={<button type="button" className="settings-text-button" onClick={() => onChange({ ...DEFAULT_APPEARANCE })}>Reset appearance</button>}>
      <div className="appearance-modes" role="radiogroup" aria-label="Color mode">
        {(["system", "light", "dark"] as const).map((option) => <label key={option} className="appearance-mode">
          <input type="radio" name="color-mode" value={option} checked={value.mode === option} onChange={() => set("mode", option)} />
          <span className={`appearance-mode-preview appearance-mode-preview--${option}`} aria-hidden="true"><i /><i /><i /></span>
          <span>{option[0].toUpperCase() + option.slice(1)}</span>
        </label>)}
      </div>
      <SettingsRow label="Accent" controlId="appearance-accent">
        <div className="appearance-color"><input id="appearance-accent" aria-label="Accent color" type="color" value={value.accent} onChange={(e) => set("accent", e.target.value)} /><span>{value.accent.toUpperCase()}</span></div>
      </SettingsRow>
      <SettingsRow label="Background" controlId="appearance-background">
        <div className="appearance-color"><input id="appearance-background" type="color" value={colors.background} onChange={(e) => set(backgroundKey, e.target.value)} /><span>{colors.background.toUpperCase()}</span></div>
      </SettingsRow>
      <SettingsRow label="Text" controlId="appearance-foreground">
        <div className="appearance-color"><input id="appearance-foreground" type="color" value={colors.foreground} onChange={(e) => set(foregroundKey, e.target.value)} /><span>{colors.foreground.toUpperCase()}</span></div>
      </SettingsRow>
      <SettingsRow label="Mode colors" description="Light and dark colors are saved separately.">
        <button type="button" className="settings-text-button" onClick={() => onChange({ ...value, [backgroundKey]: null, [foregroundKey]: null })}>Restore {mode}</button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Window size" description="Size changes apply when you save. Content scrolls at the height limit.">
      {([
        ["windowWidth", "Width", 460, 1000],
        ["chatMaxHeight", "Maximum chat height", 480, 1400],
        ["settingsMaxHeight", "Maximum settings height", 480, 1200],
      ] as const).map(([key, label, min, max]) => <SettingsRow key={key} label={label} controlId={`appearance-${key}`}>
        <div className="settings-range"><output htmlFor={`appearance-${key}`} className="settings-help">{value[key]} px</output>
          <input id={`appearance-${key}`} type="range" min={min} max={max} step="10" value={value[key]} onChange={(e) => set(key, Number(e.target.value))} />
        </div>
      </SettingsRow>)}
    </SettingsGroup>
    <SettingsGroup title="Typography">
      <SettingsRow label="Interface font" controlId="appearance-ui-font"><select id="appearance-ui-font" className="settings-control" value={value.uiFont} onChange={(e) => set("uiFont", e.target.value as Appearance["uiFont"])}>
        <option value="system">System default</option><option value="segoe-ui">Segoe UI</option><option value="arial">Arial</option><option value="georgia">Georgia</option>
      </select></SettingsRow>
      <SettingsRow label="Response font" controlId="appearance-response-font"><select id="appearance-response-font" className="settings-control" value={value.responseFont} onChange={(e) => set("responseFont", e.target.value as Appearance["responseFont"])}>
        <option value="same">Same as interface</option><option value="segoe-ui">Segoe UI</option><option value="arial">Arial</option><option value="georgia">Georgia</option>
      </select></SettingsRow>
      {([ ["uiFontSize", "Interface size", 12, 18], ["responseFontSize", "Response size", 12, 22], ["codeFontSize", "Code size", 10, 20] ] as const).map(([key, label, min, max]) => <SettingsRow key={key} label={label} controlId={`appearance-${key}`}>
        <div className="appearance-size"><input id={`appearance-${key}`} className="settings-control" type="number" min={min} max={max} value={value[key]} onChange={(e) => { const number = Number(e.target.value); if (Number.isInteger(number) && number >= min && number <= max) set(key, number); }} /><span className="settings-help">px</span></div>
      </SettingsRow>)}
    </SettingsGroup>
    <SettingsGroup title="Advanced">
      <SettingsRow label="Code font" controlId="appearance-code-font"><select id="appearance-code-font" className="settings-control" value={value.codeFont} onChange={(e) => set("codeFont", e.target.value as Appearance["codeFont"])}>
        <option value="cascadia-code">Cascadia Code</option><option value="consolas">Consolas</option><option value="courier-new">Courier New</option>
      </select></SettingsRow>
      <SettingsRow label="Reduce motion" description="Limit animations or follow your system." controlId="appearance-motion"><select id="appearance-motion" className="settings-control" value={value.reduceMotion} onChange={(e) => set("reduceMotion", e.target.value as Appearance["reduceMotion"])}>
        <option value="system">System</option><option value="on">On</option><option value="off">Off</option>
      </select></SettingsRow>
    </SettingsGroup>
    <p className="settings-help mt-3">Colors and fonts preview immediately. Save to apply window sizes and keep your changes.</p>
  </>;
}
