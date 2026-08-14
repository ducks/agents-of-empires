# Hello Service arena

This is the smallest complete Agents of Empires arena package. Three agents receive equivalent blank NixOS guests and race to deploy a service that survives service restart and host reboot.

```bash
agents-of-empires arena validate .
credentials="$(scripts/prepare-credentials.sh)"
agents-of-empires run arena.toml \
  --adapter oracle=adapters/oracle.sh \
  --credential builder-one="$credentials/builder-one.env" \
  --credential builder-two="$credentials/builder-two.env" \
  --credential builder-three="$credentials/builder-three.env"
```

Run `nix flake lock` before publishing a derived arena so every competitor receives the same pinned guest image.
