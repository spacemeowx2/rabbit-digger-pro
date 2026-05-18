import {
  ButtonItem,
  PanelSection,
  PanelSectionRow,
  Spinner,
  staticClasses,
} from "@decky/ui";
import { callable, definePlugin, toaster } from "@decky/api";
import { useEffect, useState } from "react";
import { FaNetworkWired } from "react-icons/fa";

type HelperStatus = {
  installed: boolean;
  active: boolean;
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

const getStatus = callable<[], HelperStatus>("get_status");
const checkUpdate = callable<[], HelperStatus>("check_update");
const applyUpdate = callable<[], UpdateResult>("apply_update");

function StateText({ status }: { status: HelperStatus | null }) {
  if (!status) {
    return <div>Loading...</div>;
  }

  const rows = [
    ["Helper", status.installed ? "Installed" : "Missing"],
    ["Service", status.active ? "Running" : "Stopped"],
    ["Plugin", status.plugin_version],
    ["Helper version", status.helper_version ?? "Unknown"],
    ["Latest", status.latest_version ?? "Unknown"],
  ];

  return (
    <div style={{ display: "grid", gap: "6px", fontSize: "13px" }}>
      {rows.map(([label, value]) => (
        <div
          key={label}
          style={{ display: "flex", justifyContent: "space-between", gap: "12px" }}
        >
          <span style={{ color: "#8f98a0" }}>{label}</span>
          <span>{value}</span>
        </div>
      ))}
      {status.last_error ? (
        <div style={{ color: "#ff8a8a", marginTop: "4px" }}>{status.last_error}</div>
      ) : null}
    </div>
  );
}

function Content() {
  const [status, setStatus] = useState<HelperStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = async () => {
    setStatus(await getStatus());
  };

  const onCheckUpdate = async () => {
    setBusy(true);
    try {
      const next = await checkUpdate();
      setStatus(next);
      toaster.toast({
        title: "Rabbit Digger Pro",
        body: next.update_available ? "Update available" : "Already up to date",
      });
    } catch (error) {
      toaster.toast({ title: "Update check failed", body: String(error) });
    } finally {
      setBusy(false);
    }
  };

  const onApplyUpdate = async () => {
    setBusy(true);
    try {
      const result = await applyUpdate();
      await refresh();
      if (result.ok) {
        toaster.toast({
          title: "Rabbit Digger Pro updated",
          body: result.needs_reload
            ? "Reload Decky or restart Gaming Mode to finish plugin update"
            : `Installed ${result.version ?? "latest version"}`,
        });
      } else {
        toaster.toast({
          title: "Rabbit Digger Pro update failed",
          body: result.error ?? "Unknown error",
        });
      }
    } catch (error) {
      toaster.toast({ title: "Rabbit Digger Pro update failed", body: String(error) });
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  return (
    <PanelSection title="Status">
      <PanelSectionRow>
        <StateText status={status} />
      </PanelSectionRow>
      {busy ? (
        <PanelSectionRow>
          <Spinner />
        </PanelSectionRow>
      ) : null}
      <PanelSectionRow>
        <ButtonItem layout="below" onClick={refresh}>
          Refresh
        </ButtonItem>
      </PanelSectionRow>
      <PanelSectionRow>
        <ButtonItem layout="below" onClick={onCheckUpdate}>
          Check Update
        </ButtonItem>
      </PanelSectionRow>
      <PanelSectionRow>
        <ButtonItem layout="below" onClick={onApplyUpdate}>
          Update
        </ButtonItem>
      </PanelSectionRow>
    </PanelSection>
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
