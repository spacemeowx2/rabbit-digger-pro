import {
  ButtonItem,
  PanelSection,
  PanelSectionRow,
  Spinner,
  staticClasses,
} from "@decky/ui";
import { callable, definePlugin, toaster } from "@decky/api";
import { useEffect, useState } from "react";
import {
  FaBolt,
  FaDownload,
  FaNetworkWired,
  FaPlay,
  FaRedo,
  FaSearch,
  FaStop,
} from "react-icons/fa";

type ProtectionState = "on" | "attention" | "starting" | "off";

type HelperStatus = {
  installed: boolean;
  install_mode: "system" | "user" | "missing";
  active: boolean;
  system_active: boolean;
  system_enabled: boolean;
  user_active: boolean;
  user_enabled: boolean;
  tun_active: boolean;
  tun_name: string;
  tun_addresses: string[];
  dns_active: boolean;
  dns_servers: string[];
  protection: ProtectionState;
  summary: string;
  helper_path: string;
  helper_version: string | null;
  plugin_version: string;
  update_available: boolean;
  latest_version: string | null;
  last_error: string | null;
};

type UpdateResult = {
  ok: boolean;
  version: string | null;
  needs_reload: boolean;
  error: string | null;
};

type CheckResult = {
  ok: boolean;
  dns: { ok: boolean; message: string };
  github: { ok: boolean; message: string };
  manifest: { ok: boolean; message: string };
};

type LogsResult = {
  logs: string;
};

const getStatus = callable<[], HelperStatus>("get_status");
const checkUpdate = callable<[], HelperStatus>("check_update");
const applyUpdate = callable<[], UpdateResult>("apply_update");
const startTunnel = callable<[], HelperStatus>("start_tunnel");
const stopTunnel = callable<[], HelperStatus>("stop_tunnel");
const restartTunnel = callable<[], HelperStatus>("restart_tunnel");
const testConnectivity = callable<[], CheckResult>("test_connectivity");
const getLogs = callable<[], LogsResult>("get_logs");

const stateCopy: Record<
  ProtectionState,
  { title: string; tone: string; color: string; background: string }
> = {
  on: {
    title: "Game proxy is on",
    tone: "Games are using Rabbit Digger Pro.",
    color: "#7ee787",
    background: "rgba(126, 231, 135, 0.12)",
  },
  attention: {
    title: "Needs attention",
    tone: "The tunnel is running, but DNS needs a check.",
    color: "#ffd166",
    background: "rgba(255, 209, 102, 0.13)",
  },
  starting: {
    title: "Starting",
    tone: "The service is running; the tunnel is not ready yet.",
    color: "#9ecbff",
    background: "rgba(158, 203, 255, 0.12)",
  },
  off: {
    title: "Game proxy is off",
    tone: "Games are using the normal network.",
    color: "#ff8a8a",
    background: "rgba(255, 138, 138, 0.12)",
  },
};

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", gap: "12px" }}>
      <span style={{ color: "#8f98a0" }}>{label}</span>
      <span style={{ textAlign: "right" }}>{value}</span>
    </div>
  );
}

function StatusCard({ status }: { status: HelperStatus | null }) {
  if (!status) {
    return <div style={{ minHeight: "74px" }}>Loading...</div>;
  }

  const copy = stateCopy[status.protection] ?? stateCopy.off;
  const updateText = status.latest_version
    ? status.update_available
      ? `Update to ${status.latest_version}`
      : "Up to date"
    : "Not checked";

  return (
    <div style={{ display: "grid", gap: "10px" }}>
      <div
        style={{
          padding: "12px",
          borderRadius: "6px",
          background: copy.background,
          border: `1px solid ${copy.color}`,
        }}
      >
        <div style={{ color: copy.color, fontSize: "16px", fontWeight: 700 }}>
          {copy.title}
        </div>
        <div style={{ marginTop: "4px", color: "#d8dee9", fontSize: "13px" }}>
          {status.summary || copy.tone}
        </div>
      </div>

      <div style={{ display: "grid", gap: "6px", fontSize: "13px" }}>
        <DetailRow
          label="Game traffic"
          value={status.tun_active ? "Routed" : "Normal network"}
        />
        <DetailRow label="Starts on boot" value={status.system_enabled ? "On" : "Off"} />
        <DetailRow label="DNS" value={status.dns_active ? "Attached" : "Not attached"} />
        <DetailRow label="Update" value={updateText} />
      </div>

      {status.last_error ? (
        <div style={{ color: "#ff8a8a", fontSize: "12px" }}>{status.last_error}</div>
      ) : null}
    </div>
  );
}

function ConnectivityView({ result }: { result: CheckResult | null }) {
  if (!result) {
    return null;
  }

  const rows = [
    ["DNS", result.dns],
    ["GitHub", result.github],
    ["Latest release", result.manifest],
  ] as const;

  return (
    <div style={{ display: "grid", gap: "6px", fontSize: "13px" }}>
      {rows.map(([label, item]) => (
        <DetailRow
          key={label}
          label={label}
          value={`${item.ok ? "OK" : "Failed"} - ${item.message}`}
        />
      ))}
    </div>
  );
}

function Diagnostics({ status }: { status: HelperStatus | null }) {
  if (!status) {
    return null;
  }

  return (
    <div style={{ display: "grid", gap: "6px", fontSize: "12px" }}>
      <DetailRow
        label="Control mode"
        value={status.install_mode === "system" ? "Game Mode service" : status.install_mode}
      />
      <DetailRow label="Service" value={status.system_active ? "Running" : "Stopped"} />
      <DetailRow label="Tunnel" value={status.tun_active ? status.tun_name : "Not ready"} />
      <DetailRow
        label="Address"
        value={status.tun_addresses.length ? status.tun_addresses.join(", ") : "None"}
      />
      <DetailRow
        label="Name server"
        value={status.dns_servers.length ? status.dns_servers.join(", ") : "None"}
      />
      <DetailRow label="Plugin" value={status.plugin_version} />
    </div>
  );
}

function LogView({ logs }: { logs: string | null }) {
  if (!logs) {
    return null;
  }

  return (
    <pre
      style={{
        maxHeight: "260px",
        overflow: "auto",
        whiteSpace: "pre-wrap",
        wordBreak: "break-word",
        fontSize: "11px",
        lineHeight: 1.35,
        padding: "10px",
        borderRadius: "6px",
        background: "rgba(0, 0, 0, 0.28)",
      }}
    >
      {logs}
    </pre>
  );
}

function Content() {
  const [status, setStatus] = useState<HelperStatus | null>(null);
  const [checkResult, setCheckResult] = useState<CheckResult | null>(null);
  const [logs, setLogs] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = async () => {
    setStatus(await getStatus());
  };

  const setNextStatus = (next: HelperStatus, successMessage?: string) => {
    setStatus(next);
    if (next.last_error) {
      toaster.toast({ title: "Rabbit Digger Pro", body: next.last_error });
    } else if (successMessage) {
      toaster.toast({ title: "Rabbit Digger Pro", body: successMessage });
    }
  };

  const runStatusAction = async (
    name: string,
    action: () => Promise<HelperStatus>,
    successMessage: string,
  ) => {
    setBusy(name);
    try {
      setNextStatus(await action(), successMessage);
    } catch (error) {
      toaster.toast({ title: "Rabbit Digger Pro", body: String(error) });
    } finally {
      setBusy(null);
    }
  };

  const onCheckUpdate = async () => {
    setBusy("check-update");
    try {
      const next = await checkUpdate();
      setStatus(next);
      if (next.last_error) {
        toaster.toast({ title: "Update check failed", body: next.last_error });
      } else {
        toaster.toast({
          title: "Rabbit Digger Pro",
          body: next.update_available ? "A new version is ready" : "You are up to date",
        });
      }
    } catch (error) {
      toaster.toast({ title: "Update check failed", body: String(error) });
    } finally {
      setBusy(null);
    }
  };

  const onApplyUpdate = async () => {
    setBusy("update");
    try {
      const result = await applyUpdate();
      await refresh();
      if (result.ok) {
        toaster.toast({
          title: "Rabbit Digger Pro updated",
          body: result.needs_reload
            ? "Restart Decky Loader or reopen Gaming Mode to finish the menu update"
            : `Installed ${result.version ?? "latest version"}`,
        });
      } else {
        toaster.toast({
          title: "Update failed",
          body: result.error ?? "Unknown error",
        });
      }
    } catch (error) {
      toaster.toast({ title: "Update failed", body: String(error) });
    } finally {
      setBusy(null);
    }
  };

  const onTestConnection = async () => {
    setBusy("test");
    try {
      const result = await testConnectivity();
      setCheckResult(result);
      toaster.toast({
        title: result.ok ? "Connection looks good" : "Connection needs attention",
        body: result.ok ? result.manifest.message : result.github.message,
      });
    } catch (error) {
      toaster.toast({ title: "Connection test failed", body: String(error) });
    } finally {
      setBusy(null);
    }
  };

  const onToggleLogs = async () => {
    if (logs) {
      setLogs(null);
      return;
    }
    setBusy("logs");
    try {
      const result = await getLogs();
      setLogs(result.logs || "No logs yet");
    } catch (error) {
      toaster.toast({ title: "Could not load logs", body: String(error) });
    } finally {
      setBusy(null);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const isBusy = busy !== null;
  const statusReady = status !== null;
  const protectionOn = Boolean(status?.system_active || status?.tun_active);

  return (
    <>
      <PanelSection title="Game Proxy">
        <PanelSectionRow>
          <StatusCard status={status} />
        </PanelSectionRow>
        {isBusy ? (
          <PanelSectionRow>
            <Spinner />
          </PanelSectionRow>
        ) : null}
        <PanelSectionRow>
          <ButtonItem
            layout="below"
            icon={protectionOn ? <FaRedo /> : <FaPlay />}
            disabled={isBusy || !statusReady}
            onClick={() =>
              protectionOn
                ? runStatusAction("restart", restartTunnel, "Game proxy restarted")
                : runStatusAction("start", startTunnel, "Game proxy is on")
            }
          >
            {!statusReady
              ? "Loading Status"
              : protectionOn
                ? "Restart Game Proxy"
                : "Turn On Game Proxy"}
          </ButtonItem>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem
            layout="below"
            icon={<FaStop />}
            disabled={isBusy || !protectionOn}
            onClick={() => runStatusAction("stop", stopTunnel, "Game proxy is off")}
          >
            Turn Off Game Proxy
          </ButtonItem>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Updates">
        <PanelSectionRow>
          <ButtonItem
            layout="below"
            icon={<FaSearch />}
            disabled={isBusy}
            onClick={onCheckUpdate}
          >
            Check Latest Version
          </ButtonItem>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem
            layout="below"
            icon={<FaDownload />}
            disabled={isBusy}
            onClick={onApplyUpdate}
          >
            Update Now
          </ButtonItem>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Connection">
        <PanelSectionRow>
          <ButtonItem
            layout="below"
            icon={<FaBolt />}
            disabled={isBusy}
            onClick={onTestConnection}
          >
            Test GitHub Access
          </ButtonItem>
        </PanelSectionRow>
        {checkResult ? (
          <PanelSectionRow>
            <ConnectivityView result={checkResult} />
          </PanelSectionRow>
        ) : null}
      </PanelSection>

      <PanelSection title="Details">
        <PanelSectionRow>
          <Diagnostics status={status} />
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem layout="below" disabled={isBusy} onClick={refresh}>
            Refresh Status
          </ButtonItem>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem layout="below" disabled={isBusy} onClick={onToggleLogs}>
            {logs ? "Hide Recent Logs" : "Show Recent Logs"}
          </ButtonItem>
        </PanelSectionRow>
        {logs ? (
          <PanelSectionRow>
            <LogView logs={logs} />
          </PanelSectionRow>
        ) : null}
      </PanelSection>
    </>
  );
}

export default definePlugin(() => {
  return {
    name: "Rabbit Digger Pro",
    titleView: <div className={staticClasses.Title}>Rabbit Digger Pro</div>,
    content: <Content />,
    icon: <FaNetworkWired />,
  };
});
