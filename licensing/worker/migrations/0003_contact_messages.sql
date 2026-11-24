-- Contact form (24 Nov 2026): filescape.app/contact posts to api.filescape.app/v1/contact.
-- Messages are kept here and answered from admin.filescape.app/messages, so no personal email
-- address ever appears on the site or in a reply.

CREATE TABLE messages (
  id TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL,
  name TEXT,
  email TEXT NOT NULL,
  topic TEXT NOT NULL,
  body TEXT NOT NULL,
  -- Keyed hash of the sender's IP address, only for rate limiting.
  ip_hash TEXT,
  -- 'new', 'answered' (a reply was sent) or 'closed'.
  status TEXT NOT NULL DEFAULT 'new',
  -- When the notification email to CONTACT_NOTIFY_TO went out (NULL: not set, or it failed).
  notified_at INTEGER,
  updated_at INTEGER NOT NULL
);
CREATE INDEX messages_status ON messages(status, created_at);
CREATE INDEX messages_email ON messages(email, created_at);
CREATE INDEX messages_ip ON messages(ip_hash, created_at);
CREATE INDEX messages_created ON messages(created_at);

-- Replies sent from the admin site (emailed to the sender from MAIL_FROM).
CREATE TABLE message_replies (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  message_id TEXT NOT NULL,
  at INTEGER NOT NULL,
  actor TEXT NOT NULL,
  body TEXT NOT NULL
);
CREATE INDEX message_replies_message ON message_replies(message_id, at);
