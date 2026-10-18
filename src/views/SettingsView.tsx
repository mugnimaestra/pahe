import { useEffect, useState, type FormEvent } from "react";
import { ipc, type Settings } from "../ipc/client";
import { messageOf } from "../message";

export default function SettingsView() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{
    kind: "saved" | "error";
    message: string;
  } | null>(null);

  useEffect(() => {
    let active = true;

    ipc
      .getSettings()
      .then((loaded) => {
        if (active) setSettings(loaded);
      })
      .catch((error) => {
        if (!active) return;
        setStatus({ kind: "error", message: messageOf(error) });
      });

    return () => {
      active = false;
    };
  }, []);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!settings) return;

    setSaving(true);
    setStatus(null);

    try {
      const saved = await ipc.saveSettings(settings);
      setSettings(saved);
      setStatus({ kind: "saved", message: "Settings saved." });
    } catch (error) {
      setStatus({ kind: "error", message: messageOf(error) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="view view-settings">
      <h1>Settings</h1>

      {!settings ? (
        status ? (
          <p role="alert">{status.message}</p>
        ) : (
          <p className="muted">Loading settings…</p>
        )
      ) : (
        <form className="settings-form" onSubmit={handleSubmit}>
          <label htmlFor="base-url">Base URL</label>
          <input
            id="base-url"
            type="url"
            required
            value={settings.baseUrl}
            onChange={(event) =>
              setSettings({
                ...settings,
                baseUrl: event.currentTarget.value,
              })
            }
          />
          <p className="muted">
            The animepahe origin pahe talks to. Default: https://animepahe.pw
          </p>
          <button className="button" type="submit" disabled={saving}>
            {saving ? "Saving…" : "Save"}
          </button>
          {status?.kind === "saved" && <p role="status">{status.message}</p>}
          {status?.kind === "error" && <p role="alert">{status.message}</p>}
        </form>
      )}
    </section>
  );
}
