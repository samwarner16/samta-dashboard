# Local Launchd Support (macOS)

These templates provide local service wrappers for API and worker processes on macOS.

## Install and load services

```bash
./scripts/local-services.sh install
```

Then start:

```bash
./scripts/local-services.sh start
```

Useful lifecycle commands:

- `./scripts/local-services.sh stop`
- `./scripts/local-services.sh restart`
- `./scripts/local-services.sh status`
- `./scripts/local-services.sh logs api|worker`
- `./scripts/local-services.sh logs all`

Service logs:

- `./scripts/local-services.sh logs api`
- `./scripts/local-services.sh logs worker`

Files:

- `api.plist.template`
- `worker.plist.template`
