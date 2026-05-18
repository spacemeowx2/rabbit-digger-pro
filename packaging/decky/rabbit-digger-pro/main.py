import hashlib
import json
import os
import pwd
import re
import secrets
import shutil
import socket
import subprocess
import tempfile
import urllib.request
import zipfile
from pathlib import Path
from typing import Any


PLUGIN_DIR = Path(__file__).resolve().parent
SERVICE_UNIT = "rabbit-digger-pro.service"
DEFAULT_BIND = "127.0.0.1:9091"
TUN_DEVICE = "tun-rdp"
TUN_TABLE = "2468"
SYSTEM_HELPER = Path("/var/lib/rabbit_digger_pro/bin/rabbit-digger-pro")
SYSTEM_UNIT = Path("/etc/systemd/system") / SERVICE_UNIT
DEFAULT_MANIFEST_URL = (
    "https://github.com/spacemeowx2/rabbit-digger-pro/releases/latest/download/"
    "steamdeck-update-manifest.json"
)


def _user_home() -> Path:
    if os.environ.get("DECKY_USER_HOME"):
        return Path(os.environ["DECKY_USER_HOME"])
    if os.geteuid() == 0 and Path("/home/deck").exists():
        return Path("/home/deck")
    return Path(os.environ.get("HOME") or "/home/deck")


def _xdg_data_home() -> Path:
    if os.environ.get("DECKY_USER_HOME") or (os.geteuid() == 0 and Path("/home/deck").exists()):
        return _user_home() / ".local/share"
    return Path(os.environ.get("XDG_DATA_HOME", _user_home() / ".local/share"))


def _xdg_config_home() -> Path:
    if os.environ.get("DECKY_USER_HOME") or (os.geteuid() == 0 and Path("/home/deck").exists()):
        return _user_home() / ".config"
    return Path(os.environ.get("XDG_CONFIG_HOME", _user_home() / ".config"))


def _data_dir() -> Path:
    return _xdg_data_home() / "rabbit_digger_pro"


def _config_dir() -> Path:
    return _xdg_config_home() / "rabbit_digger_pro"


def _helper_binary() -> Path:
    return _data_dir() / "helper" / "rabbit-digger-pro"


def _token_path() -> Path:
    return _data_dir() / "decky-token"


def _update_config_path() -> Path:
    return _config_dir() / "decky-update.json"


def _plugin_version() -> str:
    package_json = PLUGIN_DIR / "package.json"
    try:
        return json.loads(package_json.read_text()).get("version", "0.0.0")
    except Exception:
        return "0.0.0"


def _load_update_config() -> dict[str, Any]:
    config_path = _update_config_path()
    if not config_path.exists():
        return {"manifest_url": DEFAULT_MANIFEST_URL}
    try:
        config = json.loads(config_path.read_text())
    except Exception:
        return {"manifest_url": DEFAULT_MANIFEST_URL}
    config.setdefault("manifest_url", DEFAULT_MANIFEST_URL)
    return config


def _ensure_token() -> str:
    token_path = _token_path()
    token_path.parent.mkdir(parents=True, exist_ok=True)
    if token_path.exists():
        return token_path.read_text().strip()
    token = secrets.token_urlsafe(32)
    token_path.write_text(token)
    token_path.chmod(0o600)
    return token


def _deck_user() -> tuple[str, int]:
    try:
        info = pwd.getpwnam("deck")
        return info.pw_name, info.pw_uid
    except KeyError:
        return pwd.getpwuid(os.getuid()).pw_name, os.getuid()


def _run(args: list[str], timeout: int = 60) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            args,
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )
    except FileNotFoundError as error:
        return subprocess.CompletedProcess(args, 127, "", str(error))


def _run_as_deck(args: list[str], timeout: int = 60) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    _, uid = _deck_user()
    env["XDG_RUNTIME_DIR"] = f"/run/user/{uid}"
    env["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path=/run/user/{uid}/bus"

    command = args
    if os.geteuid() == 0:
        user, _ = _deck_user()
        command = [
            "runuser",
            "-u",
            user,
            "--",
            "env",
            f"XDG_RUNTIME_DIR={env['XDG_RUNTIME_DIR']}",
            f"DBUS_SESSION_BUS_ADDRESS={env['DBUS_SESSION_BUS_ADDRESS']}",
            *args,
        ]

    try:
        return subprocess.run(
            command,
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            env=env,
        )
    except FileNotFoundError as error:
        return subprocess.CompletedProcess(command, 127, "", str(error))


def _systemctl_system(*args: str, timeout: int = 60) -> subprocess.CompletedProcess[str]:
    return _run(["systemctl", *args], timeout=timeout)


def _systemctl_user(*args: str) -> subprocess.CompletedProcess[str]:
    return _run_as_deck(["systemctl", "--user", *args])


def _download(url: str, dest: Path) -> None:
    request = urllib.request.Request(url, headers={"User-Agent": "rabbit-digger-pro-decky"})
    with urllib.request.urlopen(request, timeout=30) as response:
        dest.write_bytes(response.read())


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _download_verified(asset: dict[str, Any], dest: Path) -> None:
    url = asset.get("url")
    expected = asset.get("sha256")
    if not url or not expected:
        raise RuntimeError("asset is missing url or sha256")
    _download(url, dest)
    actual = _sha256(dest)
    if actual.lower() != str(expected).lower():
        raise RuntimeError(f"checksum mismatch for {url}")


def _fetch_manifest() -> dict[str, Any]:
    manifest_url = _load_update_config().get("manifest_url", DEFAULT_MANIFEST_URL)
    with tempfile.TemporaryDirectory(prefix="rdp-manifest-") as temp:
        path = Path(temp) / "manifest.json"
        _download(manifest_url, path)
        return json.loads(path.read_text())


def _service_exists() -> bool:
    return _systemctl_system("cat", SERVICE_UNIT).returncode == 0


def _system_service_active() -> bool:
    return _systemctl_system("is-active", "--quiet", SERVICE_UNIT).returncode == 0


def _system_service_enabled() -> bool:
    return _systemctl_system("is-enabled", "--quiet", SERVICE_UNIT).returncode == 0


def _user_service_active() -> bool:
    return _systemctl_user("is-active", "--quiet", SERVICE_UNIT).returncode == 0


def _user_service_enabled() -> bool:
    return _systemctl_user("is-enabled", "--quiet", SERVICE_UNIT).returncode == 0


def _render_system_unit() -> str:
    return f"""[Unit]
Description=Rabbit Digger Pro
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=RUST_LOG=info
ExecStart="{SYSTEM_HELPER}" "service" "run" "--bind" "{DEFAULT_BIND}"
Restart=on-failure
RestartSec=5
KillSignal=SIGTERM
TimeoutStopSec=20

[Install]
WantedBy=multi-user.target
"""


def _ensure_system_unit() -> None:
    if os.geteuid() != 0:
        raise RuntimeError("Decky needs root access to control Game Mode protection")
    SYSTEM_UNIT.parent.mkdir(parents=True, exist_ok=True)
    current = SYSTEM_UNIT.read_text() if SYSTEM_UNIT.exists() else None
    next_unit = _render_system_unit()
    if current != next_unit:
        SYSTEM_UNIT.write_text(next_unit)
    result = _systemctl_system("daemon-reload")
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "systemd reload failed")


def _install_user_helper(helper_path: Path) -> None:
    token = _ensure_token()
    result = _run_as_deck(
        [
            str(helper_path),
            "service",
            "install-user",
            "--bind",
            DEFAULT_BIND,
            "--access-token",
            token,
            "--binary",
            str(helper_path),
        ],
        timeout=120,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "helper install failed")


def _install_system_helper(helper_path: Path) -> None:
    if os.geteuid() != 0:
        _install_user_helper(helper_path)
        return

    SYSTEM_HELPER.parent.mkdir(parents=True, exist_ok=True)
    temp_path = SYSTEM_HELPER.with_suffix(".new")
    shutil.copy2(helper_path, temp_path)
    temp_path.chmod(0o755)
    os.chown(temp_path, 0, 0)
    temp_path.replace(SYSTEM_HELPER)
    _ensure_system_unit()


def _install_plugin_zip(zip_path: Path) -> bool:
    with tempfile.TemporaryDirectory(prefix="rdp-plugin-") as temp:
        root = Path(temp)
        with zipfile.ZipFile(zip_path) as archive:
            archive.extractall(root)
        candidates = [path for path in root.iterdir() if (path / "plugin.json").exists()]
        source = candidates[0] if candidates else root
        for child in source.iterdir():
            dest = PLUGIN_DIR / child.name
            if child.is_dir():
                shutil.copytree(child, dest, dirs_exist_ok=True)
            else:
                shutil.copy2(child, dest)
    return True


def _tun_status() -> dict[str, Any]:
    result = _run(["ip", "-o", "addr", "show", "dev", TUN_DEVICE])
    if result.returncode != 0:
        return {"active": False, "name": TUN_DEVICE, "addresses": []}

    addresses = re.findall(r"\sinet6?\s+([^ ]+)", result.stdout)
    return {"active": True, "name": TUN_DEVICE, "addresses": addresses}


def _dns_status(tun: dict[str, Any]) -> dict[str, Any]:
    try:
        content = Path("/etc/resolv.conf").read_text()
    except Exception as error:
        return {"active": False, "servers": [], "message": str(error)}

    servers = []
    for line in content.splitlines():
        parts = line.strip().split()
        if len(parts) >= 2 and parts[0] == "nameserver":
            servers.append(parts[1])

    tun_ips = {
        addr.split("/", 1)[0]
        for addr in tun.get("addresses", [])
        if "." in addr and not addr.startswith("127.")
    }
    active = bool(tun.get("active")) and any(
        server in tun_ips or server.startswith("10.99.") for server in servers
    )
    rule = _run(["ip", "rule", "show"])
    dns_rule_active = (
        rule.returncode == 0
        and "dport 53" in rule.stdout
        and f"lookup {TUN_TABLE}" in rule.stdout
    )
    if bool(tun.get("active")) and dns_rule_active:
        active = True

    return {
        "active": active,
        "servers": servers,
        "message": "Captured by tunnel"
        if dns_rule_active
        else (", ".join(servers) if servers else "No DNS server found"),
    }


def _protection_state(system_active: bool, tun: dict[str, Any], dns: dict[str, Any]) -> str:
    if system_active and tun.get("active") and dns.get("active"):
        return "on"
    if system_active and tun.get("active"):
        return "attention"
    if system_active:
        return "starting"
    return "off"


def _summary_for_state(state: str) -> str:
    if state == "on":
        return "Game traffic is routed through Rabbit Digger Pro."
    if state == "attention":
        return "Tunnel is running, but DNS does not look fully attached."
    if state == "starting":
        return "Service is running, but the tunnel is not ready yet."
    return "Game traffic is not routed through Rabbit Digger Pro."


def _status_from_manifest(manifest: dict[str, Any] | None = None) -> dict[str, Any]:
    plugin_version = _plugin_version()
    latest = None
    update_available = False

    if manifest is not None:
        latest = manifest.get("version")
        update_available = bool(latest and latest != plugin_version)

    system_active = _system_service_active()
    user_active = _user_service_active()
    tun = _tun_status()
    dns = _dns_status(tun)
    state = _protection_state(system_active, tun, dns)
    system_installed = SYSTEM_HELPER.exists() or _service_exists()
    user_installed = _helper_binary().exists()

    return {
        "installed": system_installed or user_installed,
        "install_mode": "system" if system_installed else ("user" if user_installed else "missing"),
        "active": system_active or user_active,
        "system_active": system_active,
        "system_enabled": _system_service_enabled(),
        "user_active": user_active,
        "user_enabled": _user_service_enabled(),
        "tun_active": bool(tun.get("active")),
        "tun_name": tun.get("name", TUN_DEVICE),
        "tun_addresses": tun.get("addresses", []),
        "dns_active": bool(dns.get("active")),
        "dns_servers": dns.get("servers", []),
        "protection": state,
        "summary": _summary_for_state(state),
        "helper_path": str(SYSTEM_HELPER if system_installed else _helper_binary()),
        "helper_version": "installed" if system_installed or user_installed else None,
        "plugin_version": plugin_version,
        "update_available": update_available,
        "latest_version": latest,
        "last_error": None,
    }


def _require_system_helper() -> None:
    if not SYSTEM_HELPER.exists():
        raise RuntimeError("Rabbit Digger Pro is not installed for Game Mode yet")


def _start_tunnel() -> dict[str, Any]:
    _require_system_helper()
    _ensure_system_unit()
    result = _systemctl_system("enable", "--now", SERVICE_UNIT, timeout=120)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "failed to start service")
    return _status_from_manifest()


def _stop_tunnel() -> dict[str, Any]:
    result = _systemctl_system("stop", SERVICE_UNIT, timeout=60)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "failed to stop service")
    return _status_from_manifest()


def _restart_tunnel() -> dict[str, Any]:
    _require_system_helper()
    _ensure_system_unit()
    result = _systemctl_system("restart", SERVICE_UNIT, timeout=120)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "failed to restart service")
    return _status_from_manifest()


def _message_from_exception(error: Exception) -> str:
    text = str(error)
    if not text:
        return error.__class__.__name__
    return text


def _test_connectivity() -> dict[str, Any]:
    result: dict[str, Any] = {
        "ok": False,
        "dns": {"ok": False, "message": "Not tested"},
        "github": {"ok": False, "message": "Not tested"},
        "manifest": {"ok": False, "message": "Not tested"},
    }

    try:
        addresses = socket.getaddrinfo("github.com", 443, type=socket.SOCK_STREAM)
        hosts = sorted({item[4][0] for item in addresses})
        result["dns"] = {"ok": True, "message": hosts[0] if hosts else "Resolved"}
    except Exception as error:
        result["dns"] = {"ok": False, "message": _message_from_exception(error)}
        return result

    try:
        with socket.create_connection(("github.com", 443), timeout=8):
            pass
        result["github"] = {"ok": True, "message": "GitHub is reachable"}
    except Exception as error:
        result["github"] = {"ok": False, "message": _message_from_exception(error)}
        return result

    try:
        manifest = _fetch_manifest()
        version = manifest.get("version") or "latest"
        result["manifest"] = {"ok": True, "message": f"Latest release: {version}"}
        result["ok"] = True
    except Exception as error:
        result["manifest"] = {"ok": False, "message": _message_from_exception(error)}

    return result


def _journal_logs(limit: int = 80) -> str:
    result = _run(
        ["journalctl", "-u", SERVICE_UNIT, "--no-pager", "-n", str(limit), "-o", "cat"],
        timeout=15,
    )
    if result.returncode != 0:
        return result.stderr.strip() or result.stdout.strip() or "No logs available"
    return result.stdout[-12000:]


class Plugin:
    async def _main(self):
        print("Rabbit Digger Pro Decky backend initialized")

    async def get_status(self):
        return _status_from_manifest()

    async def start_tunnel(self):
        try:
            return _start_tunnel()
        except Exception as error:
            status = _status_from_manifest()
            status["last_error"] = _message_from_exception(error)
            return status

    async def stop_tunnel(self):
        try:
            return _stop_tunnel()
        except Exception as error:
            status = _status_from_manifest()
            status["last_error"] = _message_from_exception(error)
            return status

    async def restart_tunnel(self):
        try:
            return _restart_tunnel()
        except Exception as error:
            status = _status_from_manifest()
            status["last_error"] = _message_from_exception(error)
            return status

    async def check_update(self):
        try:
            manifest = _fetch_manifest()
            return _status_from_manifest(manifest)
        except Exception as error:
            status = _status_from_manifest()
            status["last_error"] = _message_from_exception(error)
            return status

    async def apply_update(self):
        try:
            manifest = _fetch_manifest()
            version = manifest.get("version")
            needs_reload = False
            with tempfile.TemporaryDirectory(prefix="rdp-update-") as temp:
                temp_dir = Path(temp)
                temp_dir.chmod(0o755)
                helper = manifest.get("helper")
                if helper:
                    helper_path = temp_dir / "rabbit-digger-pro"
                    _download_verified(helper, helper_path)
                    helper_path.chmod(0o755)
                    _install_system_helper(helper_path)
                    _systemctl_system("enable", "--now", SERVICE_UNIT, timeout=120)
                    _systemctl_system("restart", SERVICE_UNIT, timeout=120)

                decky_plugin = manifest.get("decky_plugin")
                if decky_plugin:
                    plugin_zip = temp_dir / "rabbit-digger-pro-decky.zip"
                    _download_verified(decky_plugin, plugin_zip)
                    needs_reload = _install_plugin_zip(plugin_zip)

            return {
                "ok": True,
                "version": version,
                "needs_reload": needs_reload,
                "error": None,
            }
        except Exception as error:
            return {
                "ok": False,
                "version": None,
                "needs_reload": False,
                "error": _message_from_exception(error),
            }

    async def test_connectivity(self):
        return _test_connectivity()

    async def get_logs(self):
        return {"logs": _journal_logs()}
