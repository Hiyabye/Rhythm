-- Account-lock protection of the eClass connector that survives a restart: no
-- row means automatic logins are allowed. `rejected` is set when eClass
-- rejects the login; `unrecognised` counts login responses in a row that were
-- neither success nor a known rejection (a lockout page, CAPTCHA, an error
-- page). Automatic logins stop at a rejection or three unrecognised answers in
-- a row, until the user saves the login again or forgets it. Flags and a
-- counter only: never the ID, the password or a cookie.
CREATE TABLE cybercampus_login_guard(
  id INTEGER PRIMARY KEY CHECK(id = 1),
  rejected INTEGER NOT NULL CHECK(rejected IN (0, 1)),
  unrecognised INTEGER NOT NULL CHECK(unrecognised >= 0)
);
PRAGMA user_version=45;
