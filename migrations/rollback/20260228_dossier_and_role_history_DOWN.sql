-- Rollback: 20260228_dossier_and_role_history
-- Drops dossier/role history tables and their indexes.

DROP INDEX IF EXISTS idx_person_changes_type;
DROP INDEX IF EXISTS idx_person_changes_person;
DROP TABLE IF EXISTS person_changes;

DROP INDEX IF EXISTS idx_company_changes_type;
DROP INDEX IF EXISTS idx_company_changes_company;
DROP TABLE IF EXISTS company_changes;

DROP INDEX IF EXISTS idx_dossier_supersedes;
DROP INDEX IF EXISTS idx_dossier_category;
DROP INDEX IF EXISTS idx_dossier_entity;
DROP TABLE IF EXISTS dossier_entries;

DROP INDEX IF EXISTS idx_role_history_org;
DROP INDEX IF EXISTS idx_role_history_person;
DROP TABLE IF EXISTS role_history;
