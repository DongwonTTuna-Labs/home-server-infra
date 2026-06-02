#!/usr/bin/env python3
"""Seed the codex-lb OIDC federation trust for the rs-builder review pipeline.

Run this INSIDE the codex-lb container (the app package and CODEX_LB_* env must be
importable / set). The container builds from a git context, so this host file is not
mounted — pipe it in over stdin from the repo root:

    docker compose -f stacks/codex-lb/compose.yaml exec -T codex-lb python - \
        < stacks/codex-lb/seed-oidc-trust.py

It is idempotent: it ensures exactly one GitHub Actions provider and one trust
binding that accepts the review pipeline. The binding is intentionally simple and
rename-proof: it matches only the repository and the trusted actors, NOT individual
workflow files, so splitting/renaming the workflows never breaks the exchange.
"""

from __future__ import annotations

import asyncio

from app.db.session import SessionLocal
from app.modules.api_keys.repository import ApiKeysRepository
from app.modules.api_keys.service import ApiKeysService
from app.modules.oidc.conditions import OidcConditionInput
from app.modules.oidc.repository import OidcRepository
from app.modules.oidc.service import OidcFederationService
from app.modules.usage.repository import UsageRepository

PROVIDER_NAME = "GitHub Actions"
GITHUB_ISSUER = "https://token.actions.githubusercontent.com"
AUDIENCE = "https://relay-ai.dongwontuna.net/github-actions"

BINDING_NAME = "rs-builder-relayer-client review pipeline"
REPOSITORY = "DongwonTTuna-Labs/rs-builder-relayer-client"
TRUSTED_ACTORS = '["DongwonTTuna","codex-reviewer-for-dongwonttuna[bot]"]'


async def main() -> None:
    async with SessionLocal() as session:
        repository = OidcRepository(session)
        api_keys_service = ApiKeysService(
            ApiKeysRepository(session),
            usage_repository=UsageRepository(session),
        )
        service = OidcFederationService(repository, api_keys_service)

        providers = await service.list_providers()
        provider = next((p for p in providers if p.issuer == GITHUB_ISSUER), None)
        if provider is None:
            provider = await service.create_provider(
                name=PROVIDER_NAME,
                issuer=None,
                audience=AUDIENCE,
                jwks_uri=None,
                algorithms=None,
                is_active=True,
                preset="github_actions",
            )
            print(f"created provider {provider.id} ({provider.issuer})")
        else:
            print(f"provider already present: {provider.id} ({provider.issuer})")

        bindings = await service.list_bindings(provider.id)
        if any(b.name == BINDING_NAME for b in bindings):
            print(f"binding already present: {BINDING_NAME!r} — nothing to do")
            return

        binding = await service.create_binding(
            provider.id,
            name=BINDING_NAME,
            priority=100,
            is_active=True,
            token_ttl_seconds=None,
            allowed_models=None,
            key_name_template=None,
            enforced_model=None,
            enforced_reasoning_effort=None,
            enforced_service_tier=None,
            limits=None,
            assigned_account_ids=None,
            conditions=[
                OidcConditionInput(claim="repository", operator="equals", value=REPOSITORY),
                OidcConditionInput(claim="actor", operator="in", value=TRUSTED_ACTORS),
            ],
        )
        print(f"created binding {binding.id} ({BINDING_NAME!r})")


if __name__ == "__main__":
    asyncio.run(main())
