#!/usr/bin/env python3
"""Render and verify the isolated NVIDIA Build LB Compose contract."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
COMPOSE = REPO_ROOT / "stacks" / "nvidia-build-lb" / "compose.yaml"
EXPECTED_APP_REGISTRY_DIGEST = "f457e1c3ace3e626c139a8d4c67f766fd109986a0eb497406a9de1dbd9269ee4"
EXPECTED_POSTGRES_REGISTRY_DIGEST = "c16dc18dcbad2e4aa23d27a4db72eeee28aa5d95de7c7de4020416322e35c8e5"
EXPECTED_APP_IMAGE = (
    f"ghcr.io/dongwonttuna-labs/nvidia-build-lb@sha256:{EXPECTED_APP_REGISTRY_DIGEST}"
)
EXPECTED_POSTGRES_IMAGE = (
    f"ghcr.io/dongwonttuna-labs/nvidia-build-lb@sha256:{EXPECTED_POSTGRES_REGISTRY_DIGEST}"
)
CAPABILITIES = ["CHOWN", "SETGID", "SETUID", "SETPCAP"]
DATABASE_CAPABILITIES = [*CAPABILITIES, "FOWNER", "DAC_READ_SEARCH"]
SECRET_DIR = "/opt/nvidia-build-lb/secrets"
STACK_README = REPO_ROOT / "stacks" / "nvidia-build-lb" / "README.md"
QUARANTINE_HELPER = REPO_ROOT / "scripts" / "quarantine-hermes-credentials.sh"
QUARANTINE_SENSOR = REPO_ROOT / "scripts" / "test-quarantine-hermes-credentials.sh"
DELAYED_UPDATE_WRAPPER = REPO_ROOT / "scripts" / "agent-apps-delayed-update-locked.sh"
DELAYED_UPDATE_DROP_IN = (
    REPO_ROOT
    / "stacks"
    / "nvidia-build-lb"
    / "systemd"
    / "agent-apps-delayed-update.service.d"
    / "nblb-cutover-lock.conf"
)
SECRET_TMPFS = "/run/nvidia-build-lb/secrets:rw,noexec,nosuid,nodev,size=64k,mode=0700,uid=0,gid=0"
TEMP_TMPFS = "/tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777"
EXPECTED_ENVIRONMENTS = {
    "app": {
        "NBLB_ADMIN_PUBLIC_HOST": "",
        "NBLB_DATABASE_URL": "postgres://nvidia_build_lb@db/nvidia_build_lb",
        "NBLB_REQUIRE_DOWNSTREAM_TOKEN": "1",
        "NBLB_UPSTREAM_URL": "https://integrate.api.nvidia.com/v1/chat/completions",
        "NBLB_VAULT_MASTER_KEY_FILE": "/run/nvidia-build-lb/secrets/vault_master_key",
        "NVIDIA_BUILD_LB_ADMIN_ATTEMPT_MAX_ROWS": "40000",
        "NVIDIA_BUILD_LB_ADMIN_EVENT_MAX_ROWS": "100000",
        "NVIDIA_BUILD_LB_ADMIN_LEDGER_PRUNE_BATCH_SIZE": "1000",
        "NVIDIA_BUILD_LB_PUBLIC_PORT": "2456",
    },
    "db": {
        "PGDATA": "/var/lib/postgresql/data/pgdata",
        "POSTGRES_DB": "nvidia_build_lb",
        "POSTGRES_PASSWORD_FILE": "/run/canonical-secrets/db_password",
        "POSTGRES_USER": "nvidia_build_lb",
    },
    "migrate": {
        "NBLB_DATABASE_URL": "postgres://nvidia_build_lb@db/nvidia_build_lb",
        "NVIDIA_BUILD_LB_MODE": "migrate",
    },
}
EXPECTED_LABELS = {
    "app": {
        "com.centurylinklabs.watchtower.enable": "false",
        "nvidia-build-lb.component": "gateway",
    },
    "db": {
        "com.centurylinklabs.watchtower.enable": "false",
        "nvidia-build-lb.backup-source": "true",
        "nvidia-build-lb.component": "database",
        "nvidia-build-lb.restore-isolated": "false",
    },
    "migrate": {
        "com.centurylinklabs.watchtower.enable": "false",
        "nvidia-build-lb.component": "migration",
    },
}
FORBIDDEN_RUNTIME_KEYS = {
    "pid",
    "ipc",
    "devices",
    "device_cgroup_rules",
    "privileged",
    "network_mode",
    "userns_mode",
    "uts",
    "volumes_from",
}
EXPECTED_SERVICE_KEYS = {
    "app": {
        "cap_add",
        "cap_drop",
        "command",
        "depends_on",
        "entrypoint",
        "environment",
        "image",
        "labels",
        "networks",
        "ports",
        "read_only",
        "restart",
        "secrets",
        "security_opt",
        "stop_grace_period",
        "tmpfs",
        "volumes",
    },
    "db": {
        "cap_add",
        "cap_drop",
        "command",
        "entrypoint",
        "environment",
        "healthcheck",
        "image",
        "labels",
        "networks",
        "read_only",
        "restart",
        "secrets",
        "security_opt",
        "stop_grace_period",
        "tmpfs",
        "volumes",
    },
    "migrate": {
        "cap_add",
        "cap_drop",
        "command",
        "depends_on",
        "entrypoint",
        "environment",
        "image",
        "labels",
        "networks",
        "read_only",
        "restart",
        "secrets",
        "security_opt",
        "stop_grace_period",
        "tmpfs",
    },
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def render_compose() -> dict[str, Any]:
    completed = subprocess.run(
        ["docker", "compose", "-f", str(COMPOSE), "config", "--format", "json"],
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if completed.returncode != 0:
        raise SystemExit(completed.stderr.strip() or "nvidia-build-lb compose render failed")
    rendered = json.loads(completed.stdout)
    require(isinstance(rendered, dict), "nvidia-build-lb compose root must be an object")
    return rendered


def verify_images(services: dict[str, Any]) -> None:
    app_image = services["app"].get("image")
    migrate_image = services["migrate"].get("image")
    database_image = services["db"].get("image")
    require(app_image == EXPECTED_APP_IMAGE, "application release digest drifted")
    require(migrate_image == EXPECTED_APP_IMAGE, "migration release digest drifted")
    require(database_image == EXPECTED_POSTGRES_IMAGE, "PostgreSQL release digest drifted")


def verify_service_hardening(services: dict[str, Any]) -> None:
    expected_restarts = {"app": "unless-stopped", "db": "unless-stopped", "migrate": "no"}
    expected_tmpfs = {
        "app": [
            SECRET_TMPFS,
            "/run/nvidia-build-lb/media-spool:rw,noexec,nosuid,nodev,size=256m,mode=0730,uid=0,gid=65532",
            TEMP_TMPFS,
        ],
        "migrate": [SECRET_TMPFS, TEMP_TMPFS],
        "db": [
            SECRET_TMPFS,
            "/var/run/postgresql:rw,noexec,nosuid,nodev,size=16m,mode=0775,uid=70,gid=70",
            TEMP_TMPFS,
        ],
    }
    for name, service in services.items():
        require(set(service) == EXPECTED_SERVICE_KEYS[name], f"{name} service surface drifted")
        require(
            service.get("environment") == EXPECTED_ENVIRONMENTS[name],
            f"{name} environment contract drifted",
        )
        require(service.get("read_only") is True, f"{name} root filesystem must be read-only")
        require(service.get("cap_drop") == ["ALL"], f"{name} must drop every capability")
        expected_capabilities = DATABASE_CAPABILITIES if name == "db" else CAPABILITIES
        require(
            service.get("cap_add") == expected_capabilities,
            f"{name} capability allowlist drifted",
        )
        require(
            service.get("security_opt") == ["no-new-privileges:true"],
            f"{name} must set no-new-privileges",
        )
        require("container_name" not in service, f"{name} must use project-isolated names")
        require(service.get("restart") == expected_restarts[name], f"{name} restart policy drifted")
        require(service.get("stop_grace_period") == "30s", f"{name} stop grace period drifted")
        require(service.get("tmpfs") == expected_tmpfs[name], f"{name} tmpfs hardening drifted")
        require(
            service.get("labels") == EXPECTED_LABELS[name],
            f"{name} label contract drifted",
        )
        require(
            not FORBIDDEN_RUNTIME_KEYS.intersection(service),
            f"{name} contains a forbidden runtime override",
        )
        if name == "migrate":
            require(
                service.get("command") == ["/usr/local/bin/nblb-migrate"],
                "migration command must invoke the migration binary",
            )
        else:
            require(service.get("command") is None, f"{name} command override is forbidden")
        require(service.get("entrypoint") is None, f"{name} entrypoint override is forbidden")

    require(
        services["migrate"].get("depends_on")
        == {"db": {"condition": "service_healthy", "required": True}},
        "migration dependency gate drifted",
    )
    require(
        services["app"].get("depends_on")
        == {
            "db": {"condition": "service_healthy", "required": True},
            "migrate": {"condition": "service_completed_successfully", "required": True},
        },
        "application dependency gate drifted",
    )
    require(
        services["db"].get("healthcheck")
        == {
            "test": [
                "CMD",
                "pg_isready",
                "-h",
                "127.0.0.1",
                "-U",
                "nvidia_build_lb",
                "-d",
                "nvidia_build_lb",
            ],
            "timeout": "3s",
            "interval": "10s",
            "retries": 12,
        },
        "database healthcheck drifted",
    )
    require("healthcheck" not in services["app"], "application image healthcheck must remain authoritative")
    require("healthcheck" not in services["migrate"], "migration must not add a healthcheck override")


def verify_networks_and_storage(rendered: dict[str, Any], services: dict[str, Any]) -> None:
    ports = services["app"].get("ports")
    require(
        ports
        == [
            {
                "mode": "ingress",
                "host_ip": "127.0.0.1",
                "target": 2456,
                "published": "2456",
                "protocol": "tcp",
            }
        ],
        "nvidia-build-lb must expose only loopback port 2456",
    )
    require("ports" not in services["db"], "database must not publish a host port")
    require("ports" not in services["migrate"], "migration must not publish a host port")
    require(services["db"].get("networks") == {"data": None}, "database network drifted")
    require(
        services["migrate"].get("networks") == {"data": None},
        "migration network drifted",
    )
    require(
        services["app"].get("networks") == {"data": None, "egress": None},
        "application networks drifted",
    )

    networks = rendered.get("networks", {})
    require(networks.get("data", {}).get("internal") is True, "data network must be internal")
    require(networks.get("data", {}).get("name") == "nvidia-build-lb_data", "data network name drifted")
    require(
        networks.get("egress", {}).get("name") == "nvidia-build-lb_egress",
        "egress network name drifted",
    )
    require(networks.get("egress", {}).get("internal") is not True, "egress must reach NVIDIA")

    volumes = rendered.get("volumes", {})
    require(set(volumes) == {"db-data", "vault-data"}, "unexpected nvidia-build-lb volume")
    require(
        volumes["db-data"].get("name") == "nvidia-build-lb_db-data",
        "database volume name drifted",
    )
    require(
        services["db"].get("volumes")
        == [
            {
                "type": "volume",
                "source": "db-data",
                "target": "/var/lib/postgresql/data",
                "volume": {},
            }
        ],
        "database volume mount drifted",
    )
    require(
        services["app"].get("volumes")
        == [
            {
                "type": "volume",
                "source": "vault-data",
                "target": "/var/lib/nvidia-build-lb",
                "volume": {},
            }
        ],
        "vault volume mount drifted",
    )
    require(
        volumes["vault-data"].get("name") == "nvidia-build-lb_vault-data",
        "vault volume name drifted",
    )
    require("volumes" not in services["migrate"], "migration must remain stateless")


def verify_secret_boundary(rendered: dict[str, Any], services: dict[str, Any]) -> None:
    expected_files = {
        "admin_token": f"{SECRET_DIR}/admin_token",
        "vault_master_key": f"{SECRET_DIR}/vault_master_key",
        "db_password": f"{SECRET_DIR}/db_password",
    }
    secrets = rendered.get("secrets", {})
    require(set(secrets) == set(expected_files), "nvidia-build-lb secret set drifted")
    for name, path in expected_files.items():
        require(secrets[name].get("file") == path, f"{name} host path drifted")

    expected_db_secret = [
        {
            "source": "db_password",
            "target": "/run/canonical-secrets/db_password",
            "mode": "0400",
        }
    ]
    require(services["db"].get("secrets") == expected_db_secret, "database secret mount drifted")
    require(
        services["migrate"].get("secrets") == expected_db_secret,
        "migration secret mount drifted",
    )
    require(
        services["app"].get("secrets")
        == [
            {
                "source": "admin_token",
                "target": "/run/canonical-secrets/admin_token",
                "mode": "0400",
            },
            {
                "source": "vault_master_key",
                "target": "/run/canonical-secrets/vault_master_key",
                "mode": "0400",
            },
            *expected_db_secret,
        ],
        "application secret mounts drifted",
    )

    require("POSTGRES_PASSWORD" not in services["db"].get("environment", {}), "database password leaked into environment")
    app_environment = services["app"].get("environment", {})
    require(
        app_environment.get("NVIDIA_BUILD_LB_PUBLIC_PORT") == "2456",
        "application public-port authority drifted",
    )
    require(
        app_environment.get("NBLB_VAULT_MASTER_KEY_FILE")
        == "/run/nvidia-build-lb/secrets/vault_master_key",
        "application vault-key file boundary drifted",
    )
    for service in services.values():
        environment = service.get("environment", {})
        require(
            all(
                key == "NBLB_REQUIRE_DOWNSTREAM_TOKEN"
                or not key.endswith(("_TOKEN", "_KEY", "_PASSWORD"))
                for key in environment
            ),
            "plaintext credential-shaped environment key detected",
        )


def verify_documented_operations() -> None:
    document = STACK_README.read_text(encoding="utf-8")
    normalized_document = " ".join(document.split())
    expected_fragments = (
        "gh auth refresh -h github.com -s read:packages",
        'docker --config "$registry_config" login ghcr.io',
        "--username \"$registry_user\" --password-stdin",
        'docker --config "$registry_config" compose',
        "up -d --pull never",
        "NBLB_RUNTIME_CONFIG_FILE=/etc/nvidia-build-lb/runtime.env",
        f"NBLB_APP_REGISTRY_DIGEST={EXPECTED_APP_REGISTRY_DIGEST}",
        f"NBLB_POSTGRES_REGISTRY_DIGEST={EXPECTED_POSTGRES_REGISTRY_DIGEST}",
        "d30662084e4bec4ed3eebe7ef4fb0026ef2302f2",
        "( set -Eeuo pipefail set +x",
        'checkout_head=$(git -C "$app_repo" rev-parse HEAD)',
        'checkout_status=$(git -C "$app_repo" status --porcelain=v1 --untracked-files=all)',
        'test -z "$checkout_status"',
        'project_name=$("$compose_wrapper" config --format json | jq -er \'.name\')',
        "scripts/ops/production-compose.sh",
        "A production state cutover is not a routine command in this stack",
        "Never execute a slice of this README with `sed -n`",
        "scripts/quarantine-hermes-credentials.sh",
        "Hermes credential matches after quarantine: 0",
        "Legacy quarantine generation",
        "pending retirement",
        "candidate_reconciliation_required",
        "review_and_revoke_candidate_token",
        "candidate_revoked_confirmed=true",
        'retire "$LEGACY_QUARANTINE_GENERATION" --provider-credential-revoked',
        "Hermes helper-issued cutover, rollback rehearsal, and recovery",
        "`models:read` plus `chat:write`",
        "NVIDIA_API_KEY` in `/opt/agent-apps/data/hermes/.env",
        "http://127.0.0.1:2456/v1",
        "/opt/nvidia-build-lb/hermes-cutover-backups/",
        "/opt/nvidia-build-lb/hermes-cutover-state/",
        "scripts/ops/hermes_cutover.py preflight",
        ".delayed_update_lock_guard == true",
        ".issuance_allowed == true",
        "agent-apps-delayed-update.service",
        "provider-side confirmation",
        "retirement receipt",
        "tests/operations/test_hermes_cutover.py",
        "`revoked_at` must be non-null",
        '"$helper" cycle',
        "prior-token revocation",
        "exact terminal `reapplied` journal",
        "real tool-using agent run",
        "retire that exact host-only generation",
        "retire-backup --backup-id \"$BACKUP_ID\" --provider-credential-revoked",
        ".backup_absent == true",
        'registry_config=$(mktemp -d)',
        "cleanup_registry_auth()",
        "trap cleanup_registry_auth EXIT HUP INT TERM",
        'docker --config "$registry_config" logout ghcr.io',
        'find "$registry_config" -type f -delete',
        "trap - EXIT HUP INT TERM",
    )
    for fragment in expected_fragments:
        require(
            " ".join(fragment.split()) in normalized_document,
            f"documented operations contract missing: {fragment}",
        )
    require(
        "openssl rand -hex 32 |" not in document,
        "secret bootstrap must not hide OpenSSL failure in a pipeline",
    )
    require(
        "/opt/agent-apps/data/hermes/.nblb-cutover-backup-" not in document,
        "Hermes-mounted rollback backup path is forbidden",
    )
    require(
        "cutover --token-id" not in document and "HERMES_BEARER" not in document,
        "Hermes production flow must be helper-issued without plaintext bearer input",
    )
    stop_timer = document.index('sudo systemctl stop "$timer"')
    verify_exec = document.index('systemctl show -p ExecStart "$service"')
    start_timer = document.index('sudo systemctl start "$timer"')
    require(
        stop_timer < verify_exec < start_timer,
        "delayed-update timer must stay stopped until effective ExecStart is verified",
    )
    require(
        "interlock installation failed; delayed-update timer remains stopped" in document,
        "interlock failure must leave the delayed-update timer stopped",
    )

    for marker in ("ROOT_SH", "ROOT_RUNTIME_SH"):
        start = f"sudo sh <<'{marker}'\n"
        require(document.count(start) == 1, f"{marker} bootstrap block drifted")
        shell = document.split(start, 1)[1].split(f"\n{marker}\n", 1)[0]
        completed = subprocess.run(
            ["sh", "-n"],
            check=False,
            capture_output=True,
            input=shell,
            text=True,
            timeout=5,
        )
        require(completed.returncode == 0, f"{marker} bootstrap shell syntax is invalid")


def verify_quarantine_helper() -> None:
    require(QUARANTINE_HELPER.stat().st_mode & 0o111 != 0, "quarantine helper must be executable")
    require(QUARANTINE_SENSOR.stat().st_mode & 0o111 != 0, "quarantine sensor must be executable")
    completed = subprocess.run(
        ["bash", "-n", str(QUARANTINE_HELPER)],
        check=False,
        capture_output=True,
        text=True,
        timeout=5,
    )
    require(completed.returncode == 0, "quarantine helper shell syntax is invalid")
    helper = QUARANTINE_HELPER.read_text(encoding="utf-8")
    for fragment in (
        "set -Eeuo pipefail",
        "set +x",
        "docker inspect --format",
        "Hermes credential matches after quarantine: 0",
        "Hermes legacy credential matches after quarantine: 0",
        "find -P",
        "chown -R --no-dereference",
        "flock -x 9",
        "mountinfo_path=/proc/self/mountinfo",
        "Host-only cutover root aliases a Hermes mount",
        "validate_direct_upstream_env",
        "legacy-retirements",
        "legacy-quarantine-generations",
        "report_generation_receipts",
        "write_generation_receipt pending",
        "write_retirement_receipt complete",
        'find -P "$target" -depth -delete',
        "Retired legacy quarantine generation: %s",
    ):
        require(fragment in helper, f"quarantine helper contract missing: {fragment}")
    behavior = subprocess.run(
        [str(QUARANTINE_SENSOR)],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    require(
        behavior.returncode == 0,
        "quarantine helper behavioral sensors failed",
    )


def verify_delayed_update_interlock() -> None:
    require(
        DELAYED_UPDATE_WRAPPER.stat().st_mode & 0o111 != 0,
        "delayed update lock wrapper must be executable",
    )
    wrapper = DELAYED_UPDATE_WRAPPER.read_text(encoding="utf-8")
    for fragment in (
        "flock -x 9",
        "/opt/nvidia-build-lb/hermes-cutover-state",
        "exec /opt/agent-apps/bin/check-delayed-updates --apply",
    ):
        require(fragment in wrapper, f"delayed update wrapper missing: {fragment}")
    drop_in = DELAYED_UPDATE_DROP_IN.read_text(encoding="utf-8")
    require(
        "ExecStart=/usr/local/libexec/nvidia-build-lb-agent-apps-delayed-update" in drop_in,
        "delayed update systemd interlock drifted",
    )


def main() -> None:
    rendered = render_compose()
    require(rendered.get("name") == "nvidia-build-lb", "Compose project name drifted")
    services = rendered.get("services")
    require(isinstance(services, dict), "Compose services must be an object")
    require(set(services) == {"app", "db", "migrate"}, "unexpected service topology")
    verify_images(services)
    verify_service_hardening(services)
    verify_networks_and_storage(rendered, services)
    verify_secret_boundary(rendered, services)
    verify_documented_operations()
    verify_quarantine_helper()
    verify_delayed_update_interlock()
    print("nvidia-build-lb stack contract passed")


if __name__ == "__main__":
    main()
