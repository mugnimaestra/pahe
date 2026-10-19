import { useEffect, useState, type FormEvent } from "react";
import { QUALITIES } from "../downloads";
import {
  ipc,
  type ProbeOutcome,
  type ProxyPolicy,
  type Settings,
} from "../ipc/client";
import { messageOf } from "../message";

type Notice =
  | { kind: "saved" }
  | { kind: ProbeOutcome }
  | { kind: "error"; message: string };

const POLICY_LABELS: Record<ProxyPolicy, string> = {
  auto: "Auto",
  always: "Always",
  never: "Never",
};

export default function SettingsView() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [busy, setBusy] = useState<"save" | "test" | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);

  useEffect(() => {
    let active = true;

    ipc
      .getSettings()
      .then((loaded) => {
        if (active) setSettings(loaded);
      })
      .catch((error) => {
        if (!active) return;
        setNotice({ kind: "error", message: messageOf(error) });
      });

    return () => {
      active = false;
    };
  }, []);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!settings || busy) return;

    setBusy("save");
    setNotice(null);

    try {
      const saved = await ipc.saveSettings(settings);
      setSettings(saved);
      setNotice({ kind: "saved" });
    } catch (error) {
      setNotice({ kind: "error", message: messageOf(error) });
    } finally {
      setBusy(null);
    }
  }

  async function testBaseUrl() {
    if (!settings || busy) return;

    setBusy("test");
    setNotice(null);

    try {
      setNotice({ kind: await ipc.testBaseUrl(settings.baseUrl) });
    } catch (error) {
      setNotice({ kind: "error", message: messageOf(error) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="view view-settings">
      <h1>Settings</h1>

      {settings === null ? (
        notice?.kind === "error" ? (
          <p role="alert">{notice.message}</p>
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
              setSettings({ ...settings, baseUrl: event.currentTarget.value })
            }
          />
          <button
            type="button"
            onClick={testBaseUrl}
            disabled={busy !== null}
          >
            {busy === "test" ? "Testing…" : "Test"}
          </button>
          <p className="muted">
            The animepahe origin pahe talks to. Test checks whether the site or
            this machine's DNS answers for it.
          </p>

          <label htmlFor="library-folder">Library folder</label>
          <input
            id="library-folder"
            required
            value={settings.libraryFolder}
            onChange={(event) =>
              setSettings({
                ...settings,
                libraryFolder: event.currentTarget.value,
              })
            }
          />
          <p className="muted">
            Downloaded Episodes are adopted from here. Saving re-scans the
            folder.
          </p>

          <label htmlFor="default-quality">Default Quality</label>
          <select
            id="default-quality"
            value={settings.defaultQuality}
            onChange={(event) =>
              setSettings({
                ...settings,
                defaultQuality: event.currentTarget.value,
              })
            }
          >
            {QUALITIES.map((quality) => (
              <option key={quality} value={quality}>
                {quality}
              </option>
            ))}
          </select>
          <p className="muted">New Download jobs start from this Quality.</p>

          <label htmlFor="proxy-policy">Proxy policy</label>
          <select
            id="proxy-policy"
            value={settings.proxy}
            onChange={(event) =>
              setSettings({
                ...settings,
                proxy: event.currentTarget.value as ProxyPolicy,
              })
            }
          >
            {Object.entries(POLICY_LABELS).map(([policy, label]) => (
              <option key={policy} value={policy}>
                {label}
              </option>
            ))}
          </select>
          <p className="muted">
            Auto starts pahe's local proxy only when the DNS probe finds a
            Poisoned resolver. Always forces it; Never keeps it off.
          </p>

          {notice?.kind === "saved" && <p role="status">Settings saved.</p>}
          {notice?.kind === "clean" && (
            <p role="status">
              Reachable — the certificate matches this Base URL.
            </p>
          )}
          {notice?.kind === "poisoned" && (
            <p role="alert">
              Poisoned resolver: the address answered for this Base URL does
              not belong to it. pahe tunnels through its local proxy when the
              Proxy policy is Auto or Always.
            </p>
          )}
          {notice?.kind === "unreachable" && (
            <p role="alert">
              No answer from this Base URL. The site may be down, or this
              machine is offline.
            </p>
          )}
          {notice?.kind === "error" && <p role="alert">{notice.message}</p>}

          <button className="button" type="submit" disabled={busy !== null}>
            {busy === "save" ? "Saving…" : "Save"}
          </button>
        </form>
      )}
    </section>
  );
}
