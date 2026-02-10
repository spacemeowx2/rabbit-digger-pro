# Local Hysteria v2 server (for dev)

This folder contains a minimal server config and helper scripts for running a **local** Hysteria v2
server for integration testing.

## Generate a self-signed cert

```bash
cd agent-docs/hysteria
./gen-cert.sh
```

This creates `cert.pem` and `key.pem` (ignored by git).

## Start server

```bash
hysteria server -c agent-docs/hysteria/server.yaml
```

Default settings in `server.yaml`:
- listen: `127.0.0.1:18443`
- auth password: `test-password`

