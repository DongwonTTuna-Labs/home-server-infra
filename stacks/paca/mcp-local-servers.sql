WITH local_mcp_servers(server_name, url) AS (
    VALUES
        ('lsp', 'http://mcp-suite:8301/mcp'),
        ('codegraph', 'http://mcp-suite:8302/mcp'),
        ('agbrowse', 'http://mcp-suite:8303/mcp')
), active_agents AS (
    SELECT id
    FROM agents
    WHERE deleted_at IS NULL
), upserted AS (
    INSERT INTO agent_mcp_servers (
        agent_id,
        server_name,
        transport,
        command,
        args,
        url,
        env,
        is_enabled,
        created_at,
        updated_at
    )
    SELECT
        active_agents.id,
        local_mcp_servers.server_name,
        'http',
        NULL,
        '[]'::jsonb,
        local_mcp_servers.url,
        '{}'::jsonb,
        TRUE,
        NOW(),
        NOW()
    FROM active_agents
    CROSS JOIN local_mcp_servers
    ON CONFLICT (agent_id, server_name) DO UPDATE
    SET transport = EXCLUDED.transport,
        command = EXCLUDED.command,
        args = EXCLUDED.args,
        url = EXCLUDED.url,
        env = EXCLUDED.env,
        is_enabled = TRUE,
        updated_at = NOW()
    RETURNING 1
)
SELECT count(*) AS mcp_local_servers_upserted
FROM upserted;
