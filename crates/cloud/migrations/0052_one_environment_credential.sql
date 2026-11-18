-- Keep exactly one credential row per linked desktop app.
--
-- Linking used to revoke the previous credential and insert a new row, and
-- the legacy service never revoked at all, so a single app could hold
-- thousands of rows (only the newest is ever presented). Credentials are
-- stored as hashes, but keeping old ones valid or around has no use.
--
-- Keep each environment's newest unrevoked credential, delete everything else,
-- and make `environment_id` unique so linking replaces the row in place.
DELETE FROM api_environment_credentials WHERE revoked_at IS NOT NULL;

DELETE FROM api_environment_credentials c
 USING (
   SELECT credential_id,
          row_number() OVER (PARTITION BY environment_id
                             ORDER BY created_at DESC, credential_id DESC) AS rank
     FROM api_environment_credentials
 ) ranked
 WHERE c.credential_id = ranked.credential_id AND ranked.rank > 1;

CREATE UNIQUE INDEX IF NOT EXISTS api_environment_credentials_environment_unique
  ON api_environment_credentials (environment_id);
