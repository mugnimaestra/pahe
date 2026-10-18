import { useEffect, useState, type FormEvent } from "react";
import { ipc } from "../ipc/client";

function messageOf(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Something went wrong.";
}

export default function SettingsView() {
  const [loaded, setLoaded] = useState(false);
  const [baseUrl, setBaseUrl] = useState("");
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{
    kind: "saved" | "error";
    message: string;
  } | null>(null);

  useEffect(() => {
    let active = true;

    ipc
      .getSettings()
      .then((settings) => {
        if (!active) return;
        setBaseUrl(settings.baseUrl);
        setLoaded(true);
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
    setSaving(true);
    setStatus(null);

    try {
      const saved = await ipc.saveSettings({ baseUrl });
      setBaseUrl(saved.baseUrl);
      setStatus({ kind: "saved", message: "Settings saved." });
    } catch (error) {
      setStatus({ kind: "error", message: messageOf(error) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="view">
      <h1>Settings</h1>

      {!loaded ? (
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
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.currentTarget.value)}
          />
          <p className="muted">
            The animepahe origin pahe talks to. Default: https://animepahe.pw
          </p>
          <button type="submit" disabled={saving}>
            {saving ? "Saving…" : "Save"}
          </button>
          {status?.kind === "saved" && <p role="status">{status.message}</p>}
          {status?.kind === "error" && <p role="alert">{status.message}</p>}
        </form>
      )}
    </section>
  );
}
