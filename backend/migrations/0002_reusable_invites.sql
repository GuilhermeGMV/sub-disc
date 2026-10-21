-- Existing consumed links stay invalid; new links allow five registrations.
ALTER TABLE invites ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0 CHECK(use_count BETWEEN 0 AND 5);
UPDATE invites SET use_count = 5 WHERE used_by IS NOT NULL;
