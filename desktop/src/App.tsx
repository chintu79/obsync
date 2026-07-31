import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

type View = "dashboard" | "devices" | "conflicts" | "settings";

interface VaultInfo {
  path: string;
  file_count: number;
  state: string;
}

interface DeviceInfo {
  device_id: string;
  device_name: string;
  fingerprint: string;
}

interface FileEntry {
  path: string;
  size: number;
  modified_at: number;
  sync_state: string;
}

export default function App() {
  const [view, setView] = useState<View>("dashboard");
  const [vault, setVault] = useState<VaultInfo | null>(null);
  const [device, setDevice] = useState<DeviceInfo | null>(null);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [vaultPath, setVaultPath] = useState("");

  const refresh = useCallback(async () => {
    try {
      const info = await invoke<VaultInfo>("get_vault_info");
      setVault(info);
    } catch { /* no vault selected */ }
    try {
      const d = await invoke<DeviceInfo>("get_device_info");
      setDevice(d);
    } catch { /* no identity */ }
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  const handleSelectVault = async () => {
    if (!vaultPath) return;
    try {
      const info = await invoke<VaultInfo>("select_vault", { path: vaultPath });
      setVault(info);
      await refresh();
    } catch (e) {
      alert(String(e));
    }
  };

  const loadFiles = async () => {
    try {
      const list = await invoke<FileEntry[]>("get_file_list");
      setFiles(list);
    } catch { /* no vault */ }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", minHeight: "100vh" }}>
      <Header view={view} onViewChange={setView} />
      <main style={{ flex: 1, padding: "24px", maxWidth: 720, margin: "0 auto", width: "100%" }}>
        {view === "dashboard" && (
          <Dashboard
            vault={vault}
            device={device}
            onRefresh={refresh}
            vaultPath={vaultPath}
            onVaultPathChange={setVaultPath}
            onSelectVault={handleSelectVault}
            onLoadFiles={loadFiles}
            files={files}
          />
        )}
        {view === "devices" && <Devices device={device} />}
        {view === "conflicts" && <Conflicts />}
        {view === "settings" && <Settings vault={vault} device={device} />}
      </main>
    </div>
  );
}

function Header({ view, onViewChange }: { view: View; onViewChange: (v: View) => void }) {
  const tabs: { id: View; label: string }[] = [
    { id: "dashboard", label: "Dashboard" },
    { id: "devices", label: "Devices" },
    { id: "conflicts", label: "Conflicts" },
    { id: "settings", label: "Settings" },
  ];
  return (
    <header style={{ borderBottom: "1px solid var(--border)", padding: "0 24px" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 32, height: 48 }}>
        <strong style={{ fontSize: 16, letterSpacing: "-0.02em" }}>Obsync</strong>
        <nav style={{ display: "flex", gap: 4 }}>
          {tabs.map((t) => (
            <button
              key={t.id}
              onClick={() => onViewChange(t.id)}
              style={{
                background: view === t.id ? "var(--accent)" : "transparent",
                color: view === t.id ? "#fff" : "var(--text-secondary)",
                border: "none",
                padding: "6px 12px",
                borderRadius: 6,
                cursor: "pointer",
                fontSize: 13,
                fontWeight: 500,
              }}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </div>
    </header>
  );
}

function Dashboard({
  vault, device, onRefresh, vaultPath, onVaultPathChange, onSelectVault, onLoadFiles, files,
}: {
  vault: VaultInfo | null;
  device: DeviceInfo | null;
  onRefresh: () => void;
  vaultPath: string;
  onVaultPathChange: (p: string) => void;
  onSelectVault: () => void;
  onLoadFiles: () => void;
  files: FileEntry[];
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
      {!vault ? (
        <Card>
          <h2 style={{ fontSize: 16, fontWeight: 600, marginBottom: 12 }}>Select Vault</h2>
          <div style={{ display: "flex", gap: 8 }}>
            <input
              type="text"
              placeholder="~/Documents/SecondBrain"
              value={vaultPath}
              onChange={(e) => onVaultPathChange(e.target.value)}
              style={{
                flex: 1, padding: "8px 12px", borderRadius: 6, border: "1px solid var(--border)",
                background: "var(--surface)", color: "var(--text)", fontSize: 13,
              }}
            />
            <button onClick={onSelectVault} style={btnStyle}>
              Open Vault
            </button>
          </div>
        </Card>
      ) : (
        <>
          <Card>
            <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
              <div style={{
                width: 10, height: 10, borderRadius: "50%",
                background: vault.state === "Idle" ? "var(--green)" : "var(--amber)",
              }} />
              <div>
                <div style={{ fontWeight: 600, fontSize: 15 }}>
                  {vault.path.split("/").pop() || "Vault"}
                </div>
                <div style={{ color: "var(--text-secondary)", fontSize: 13 }}>
                  {vault.file_count} files · {vault.state}
                </div>
              </div>
            </div>
          </Card>

          <Card>
            <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 12 }}>
              <h2 style={{ fontSize: 15, fontWeight: 600 }}>Devices</h2>
            </div>
            {device && (
              <div style={{
                display: "flex", alignItems: "center", justifyContent: "space-between",
                padding: "10px 12px", borderRadius: 6, background: "var(--bg)",
              }}>
                <div>
                  <div style={{ fontWeight: 500 }}>{device.device_name}</div>
                  <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
                    {device.fingerprint}
                  </div>
                </div>
                <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                  <div style={{ width: 8, height: 8, borderRadius: "50%", background: "var(--green)" }} />
                  <span style={{ fontSize: 12, color: "var(--green)" }}>Ready</span>
                </div>
              </div>
            )}
          </Card>

          <Card>
            <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 12 }}>
              <h2 style={{ fontSize: 15, fontWeight: 600 }}>Recent Files</h2>
              <button onClick={onLoadFiles} style={smallBtnStyle}>
                Refresh
              </button>
            </div>
            {files.length === 0 ? (
              <p style={{ color: "var(--text-secondary)", fontSize: 13 }}>Click Refresh to load file list</p>
            ) : (
              <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
                {files.slice(0, 10).map((f) => (
                  <div key={f.path} style={{
                    display: "flex", justifyContent: "space-between", padding: "4px 0",
                    fontSize: 13, borderBottom: "1px solid var(--border)",
                  }}>
                    <span>{f.path}</span>
                    <span style={{ color: "var(--text-secondary)" }}>
                      {f.size < 1024 ? `${f.size} B` : `${(f.size / 1024).toFixed(1)} KB`}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </Card>

          <div style={{ display: "flex", gap: 8 }}>
            <button onClick={onRefresh} style={btnStyle}>Refresh</button>
          </div>
        </>
      )}
    </div>
  );
}

function Devices({ device }: { device: DeviceInfo | null }) {
  return (
    <Card>
      <h2 style={{ fontSize: 16, fontWeight: 600, marginBottom: 16 }}>Paired Devices</h2>
      {device ? (
        <div style={{
          padding: "12px 16px", borderRadius: 8, background: "var(--bg)",
          display: "flex", alignItems: "center", justifyContent: "space-between",
        }}>
          <div>
            <div style={{ fontWeight: 500 }}>{device.device_name}</div>
            <div style={{ fontSize: 12, color: "var(--text-secondary)", fontFamily: "monospace" }}>
              {device.device_id}
            </div>
          </div>
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>
            {device.fingerprint}
          </div>
        </div>
      ) : (
        <p style={{ color: "var(--text-secondary)", fontSize: 13 }}>No device identity created yet.</p>
      )}
    </Card>
  );
}

function Conflicts() {
  return (
    <Card>
      <h2 style={{ fontSize: 16, fontWeight: 600, marginBottom: 16 }}>Conflicts</h2>
      <p style={{ color: "var(--text-secondary)", fontSize: 13 }}>No conflicts.</p>
    </Card>
  );
}

function Settings({ vault, device }: { vault: VaultInfo | null; device: DeviceInfo | null }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      <Card>
        <h2 style={{ fontSize: 16, fontWeight: 600, marginBottom: 12 }}>Settings</h2>
        <div style={{ display: "flex", flexDirection: "column", gap: 8, fontSize: 13 }}>
          <div><strong>Vault path:</strong> {vault?.path || "Not set"}</div>
          <div><strong>Files indexed:</strong> {vault?.file_count || 0}</div>
          <div><strong>Device ID:</strong> {device?.device_id || "N/A"}</div>
          <div><strong>Fingerprint:</strong> {device?.fingerprint || "N/A"}</div>
        </div>
      </Card>
      <Card>
        <h2 style={{ fontSize: 14, fontWeight: 600, marginBottom: 8 }}>About</h2>
        <p style={{ fontSize: 13, color: "var(--text-secondary)", lineHeight: 1.6 }}>
          Obsync v0.1.0<br />
          Local-first P2P vault sync.<br />
          No cloud. No accounts. Encrypted.
        </p>
      </Card>
    </div>
  );
}

function Card({ children }: { children: React.ReactNode }) {
  return (
    <div style={{
      background: "var(--surface)", border: "1px solid var(--border)", borderRadius: 10,
      padding: 16,
    }}>
      {children}
    </div>
  );
}

const btnStyle: React.CSSProperties = {
  background: "var(--accent)", color: "#fff", border: "none",
  padding: "8px 16px", borderRadius: 6, cursor: "pointer", fontSize: 13,
  fontWeight: 500,
};

const smallBtnStyle: React.CSSProperties = {
  background: "transparent", color: "var(--accent)", border: "1px solid var(--border)",
  padding: "4px 10px", borderRadius: 5, cursor: "pointer", fontSize: 12,
};
