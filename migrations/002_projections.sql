-- Projections for dashboard read models
CREATE TABLE IF NOT EXISTS agent_runs_projection (
    run_id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL,
    status VARCHAR(50) NOT NULL,
    total_cost DOUBLE PRECISION DEFAULT 0.0,
    effort_points INTEGER DEFAULT 0,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL
);

CREATE TABLE IF NOT EXISTS work_items_projection (
    item_id UUID PRIMARY KEY,
    run_id UUID NOT NULL,
    status VARCHAR(50) NOT NULL,
    assigned_agent_id UUID
);
