-- Runtime settings saved from the admin page. A setting without a row uses its default,
-- and the environment and the YAML file override both.
CREATE TABLE settings (
    key        text PRIMARY KEY,
    value      jsonb       NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    -- The user who saved it; null for changes made by the system.
    updated_by uuid
);

-- Every change to a setting, with the values that applied before and after. Secret values
-- are recorded as "[redacted]", never in the clear.
CREATE TABLE settings_audit (
    id         uuid PRIMARY KEY, -- UUIDv7, so ids sort by time
    key        text        NOT NULL,
    old_value  jsonb       NOT NULL,
    new_value  jsonb       NOT NULL,
    changed_at timestamptz NOT NULL DEFAULT now(),
    changed_by uuid
);

CREATE INDEX settings_audit_key ON settings_audit (key, id);
