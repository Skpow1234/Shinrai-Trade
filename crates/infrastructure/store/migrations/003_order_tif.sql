-- Market orders + time-in-force on durable OMS rows.

ALTER TABLE orders DROP CONSTRAINT IF EXISTS orders_order_type_check;
ALTER TABLE orders
    ADD CONSTRAINT orders_order_type_check
    CHECK (order_type IN ('Limit', 'Market'));

ALTER TABLE orders
    ADD COLUMN IF NOT EXISTS time_in_force TEXT NOT NULL DEFAULT 'GTC';

ALTER TABLE orders DROP CONSTRAINT IF EXISTS orders_time_in_force_check;
ALTER TABLE orders
    ADD CONSTRAINT orders_time_in_force_check
    CHECK (time_in_force IN ('GTC', 'IOC', 'FOK'));
