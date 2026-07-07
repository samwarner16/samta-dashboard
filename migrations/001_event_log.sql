CREATE TABLE IF NOT EXISTS event_log (
    id UUID PRIMARY KEY,
    occurred_at TIMESTAMP WITH TIME ZONE NOT NULL,
    actor_id UUID NOT NULL,
    resource_id UUID NOT NULL,
    correlation_id UUID,
    causation_id UUID,
    revision INTEGER NOT NULL,
    metadata JSONB,
    event_type VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_event_log_resource ON event_log(resource_id, revision);
