CREATE TABLE accounts (
  id BIGINT PRIMARY KEY,
  email TEXT NOT NULL
);

CREATE INDEX accounts_email_idx ON accounts (email);

CREATE VIEW active_accounts AS SELECT id FROM accounts;

CREATE FUNCTION account_count() RETURNS BIGINT AS $$
  SELECT count(*) FROM accounts;
$$ LANGUAGE sql;
