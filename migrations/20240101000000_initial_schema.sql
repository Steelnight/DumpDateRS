-- Users table
CREATE TABLE users (
    id INTEGER PRIMARY KEY, -- Telegram Chat ID
    location_id TEXT NOT NULL,
    notify_time TEXT NOT NULL DEFAULT '18:00',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Subscriptions table
CREATE TABLE subscriptions (
    user_id INTEGER NOT NULL,
    waste_type TEXT NOT NULL,
    PRIMARY KEY (user_id, waste_type),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

-- Pickup events table
CREATE TABLE pickup_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    location_id TEXT NOT NULL,
    date DATE NOT NULL,
    waste_type TEXT NOT NULL,
    UNIQUE(location_id, date, waste_type)
);
