-- Install in the authoritative Singapore identity database as its operator.
CREATE OR REPLACE VIEW psms_admin_subjects AS
 SELECT p.id FROM players p JOIN player_roles r ON r.player_id=p.id
 WHERE r.role='system_admin' AND NOT p.is_bot;
REVOKE ALL ON psms_admin_subjects FROM PUBLIC;
GRANT SELECT ON psms_admin_subjects TO psms_roles;
