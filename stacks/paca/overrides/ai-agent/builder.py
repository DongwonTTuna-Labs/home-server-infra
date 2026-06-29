"""Builders for LLM, skill, and MCP configuration objects."""

from __future__ import annotations

import logging
from typing import Final

from openhands.sdk import LLM
from openhands.sdk.context import KeywordTrigger, Skill
from pydantic import SecretStr

from .. import llm_catalog
from ..config import settings
from ..models.agent import AgentConfig, AgentMCPServerRow, AgentSkillRow

logger = logging.getLogger(__name__)

RELAY_REASONING_EFFORT: Final = "xhigh"


def build_llm(agent_config: AgentConfig) -> LLM:
    """Construct an OpenHands SDK LLM instance from agent configuration."""
    provider = agent_config.llm_provider
    llm_base_url = agent_config.llm_base_url or None

    # For providers outside our own catalog (freeform "Custom..." entries), route
    # through the OpenAI-compatible client by prefixing with "openai/". Checking
    # against our catalog rather than litellm.provider_list matters: litellm
    # reserves names like "custom"/"vllm"/"huggingface" for its own non-chat,
    # non-OpenAI-compatible handlers, and a user typing one of those by
    # coincidence would otherwise get silently misrouted into them.
    if llm_base_url and provider not in llm_catalog.load():
        model_str = (
            agent_config.llm_model
            if agent_config.llm_model.startswith("openai/")
            else f"openai/{agent_config.llm_model}"
        )
    else:
        model_str = f"{provider}/{agent_config.llm_model}"

    key_val = agent_config.llm_api_key_secret_ref or ""
    logger.info(
        "LLM config - model=%s base_url=%s reasoning_effort=%s api_key_set=%s",
        model_str,
        llm_base_url or "(none)",
        RELAY_REASONING_EFFORT,
        bool(key_val),
    )
    return LLM(
        model=model_str,
        api_key=SecretStr(key_val),
        base_url=llm_base_url,
        stream=True,
        reasoning_effort=RELAY_REASONING_EFFORT,
    )


def build_skills(db_skills: list[AgentSkillRow]) -> list[Skill]:
    """Convert DB skill rows into OpenHands SDK Skill objects."""
    result = []
    for row in db_skills:
        if not row.is_enabled:
            continue
        trigger = KeywordTrigger(keywords=row.triggers) if row.triggers else None
        content = row.skill_content or ""
        result.append(Skill(name=row.skill_name, content=content, trigger=trigger))
    return result


def build_mcp_config(
    db_servers: list[AgentMCPServerRow],
    agent_id: str,
    project_id: str,
) -> dict:
    """Build the MCP server configuration dict for the OpenHands SDK.

    User-configured servers come first; the built-in Paca MCP server is always
    appended last so it cannot be overridden by user entries.
    """
    servers: dict = {}

    for row in db_servers:
        if not row.is_enabled:
            continue
        if row.transport == "stdio":
            servers[row.server_name] = {
                "command": row.command,
                "args": row.args,
                "env": row.env,
            }
        else:
            servers[row.server_name] = {
                "url": row.url,
                **({"auth": "oauth"} if row.transport == "oauth" else {}),
            }

    if settings.paca_api_key:
        if settings.dev_mcp_path:
            command, args = "node", [settings.dev_mcp_path]
        else:
            command, args = "npx", ["-y", "@paca-ai/paca-mcp"]
        servers["paca"] = {
            "command": command,
            "args": args,
            "env": {
                "PACA_API_KEY": settings.paca_api_key,
                "PACA_API_URL": settings.api_base_url,
                "PACA_GATEWAY_URL": settings.gateway_base_url,
                "PACA_AGENT_ID": agent_id,
                "PACA_PROJECT_ID": project_id,
            },
        }

    return {"mcpServers": servers}
