import hashlib
import json
import os
import pwd
import secrets
import shutil
import subprocess
import tempfile
import urllib.request
import zipfile
from pathlib import Path
from typing import Any


PLUGIN_DIR = Path(__file__).resolve().parent
SERVICE_UNIT = "rabbit-digger-pro.service"
DEFAULT_BIND = "127.0.0.1:9091"
DEFAULT_MANIFEST_URL = (
    "https://github.com/spacemeowx2/rabbit-digger-pro/releases/latest/download/"
    "steamdeck-update-manifest.json"
)


def _user_home() -> Path:
    value = os.environ.get("DECKY_USER_HOME") or os.environ.get("HOME")
    return Path(value or "/home/deck")


def _xdg_data_home() -> Path:
    if os.environ.get("DECKY_USER_HOME"):
        return _user_home() / ".local/share"
    return Path(os.environ.get("XDG_DATA_HOME", _user_home() / ".local/share"))


def _xdg_config_home() -> Path:
    if os.environ.get("DECKY_USER_HOME"):
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

    return subprocess.run(
        command,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        env=env,
    )


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


def _service_active() -> bool:
    result = _systemctl_user("is-active", "--quiet", SERVICE_UNIT)
    return result.returncode == 0


def _install_helper(helper_path: Path) -> None:
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


def _status_from_manifest(manifest: dict[str, Any] | None = None) -> dict[str, Any]:
    plugin_version = _plugin_version()
    latest = None
    update_available = False
    last_error = None

    if manifest is not None:
        latest = manifest.get("version")
        update_available = bool(latest and latest != plugin_version)

    return {
        "installed": _helper_binary().exists(),
        "active": _service_active(),
        "helper_version": "installed" if _helper_binary().exists() else None,
        "plugin_version": plugin_version,
        "update_available": update_available,
        "latest_version": latest,
        "last_error": last_error,
    }


class Plugin:
    async def _main(self):
        print("Rabbit Digger Pro Decky backend initialized")

    async def get_status(self):
        return _status_from_manifest()

    async def check_update(self):
        try:
            manifest = _fetch_manifest()
            return _status_from_manifest(manifest)
        except Exception as error:
            status = _status_from_manifest()
            status["last_error"] = str(error)
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
                    _install_helper(helper_path)
                    _systemctl_user("restart", SERVICE_UNIT)

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
                "error": str(error),
            }
