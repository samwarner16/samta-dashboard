These are local dev service templates for Linux/systemd workflows.

1) Copy templates and replace `<REPO_DIR>` with your checkout path.
2) Copy into `~/.config/systemd/user/` (or `/etc/systemd/system` as root).
3) Reload systemd and start services:

```bash
systemctl --user daemon-reload
systemctl --user enable --now go-ahead-and-call-api.service go-ahead-and-call-worker.service
```

or use the helper installer (replaces `<REPO_DIR>` automatically):

```bash
./scripts/systemd/install-templates.sh
```


Suggested naming:
- `go-ahead-and-call-api.service`
- `go-ahead-and-call-worker.service`
