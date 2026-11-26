-- Every feature beyond the core shell becomes a built-in extension. Cybercampus
-- gets its own extension instead of sharing the notice extension. Upgraded
-- profiles keep what they already had: Cybercampus stays on only where it could
-- already sync, and the formerly always-present features stay on. `migrate`
-- turns all of them off for a profile created from scratch.
INSERT INTO extension_preferences(extension_id,enabled)
SELECT 'cybercampus', EXISTS(SELECT 1 FROM sources WHERE id='cybercampus' AND subscribed=1)
  AND EXISTS(SELECT 1 FROM extension_preferences WHERE extension_id='sogang_notices' AND enabled=1)
ON CONFLICT(extension_id) DO NOTHING;
INSERT INTO extension_preferences(extension_id,enabled)
VALUES('search',1),('saved',1),('recommendations',1),('week',1),('ai',1),('proactive',1)
ON CONFLICT(extension_id) DO NOTHING;
UPDATE source_extensions SET extension_id='cybercampus' WHERE source_id='cybercampus';
PRAGMA user_version=20;
