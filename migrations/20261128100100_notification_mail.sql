-- The mail of notifications as they come, for those who want a mail for each: one mail at a
-- time for each person. While one is due, those that come after it go in it, or in the one
-- after; and no one gets one sooner than a while after their last.
CREATE TABLE notification_mail (
    user_id uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- When the mail that waits is due; unset while none waits.
    due_at  timestamptz,
    -- When the last one went.
    sent_at timestamptz
);
