-- Addresses a draft e-invoice (IRP schema 1.1) needs and Zyvor did not keep: the seller's and each buyer's.
ALTER TABLE tenant_settings ADD COLUMN gst_address1 TEXT, ADD COLUMN gst_location TEXT, ADD COLUMN gst_pincode TEXT;
ALTER TABLE customers ADD COLUMN address1 TEXT, ADD COLUMN location TEXT, ADD COLUMN pincode TEXT;
