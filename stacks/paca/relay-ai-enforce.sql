CREATE OR REPLACE FUNCTION enforce_paca_relay_llm()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    NEW.llm_provider := 'ai-relay';
    NEW.llm_model := 'gpt-5.5';
    NEW.llm_base_url := 'https://relay-ai.dongwontuna.net/v1';
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_enforce_paca_relay_llm ON agents;

CREATE TRIGGER trg_enforce_paca_relay_llm
BEFORE INSERT OR UPDATE
ON agents
FOR EACH ROW
EXECUTE FUNCTION enforce_paca_relay_llm();

ALTER TABLE agents DROP CONSTRAINT IF EXISTS agents_relay_ai_llm_only;

UPDATE agents
SET llm_provider = 'ai-relay',
    llm_model = 'gpt-5.5',
    llm_base_url = 'https://relay-ai.dongwontuna.net/v1'
WHERE deleted_at IS NULL;

ALTER TABLE agents ADD CONSTRAINT agents_relay_ai_llm_only
CHECK (
    llm_provider = 'ai-relay'
    AND llm_model = 'gpt-5.5'
    AND llm_base_url = 'https://relay-ai.dongwontuna.net/v1'
);
