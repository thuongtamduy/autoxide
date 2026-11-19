- Module `inventory`: back-in-stock alerts per store — `POST /api/v1/inventory/public/restock-alerts` (separate
  consent, anti-spam, two-step confirmation through `AUTOXIDE_INVENTORY_RESTOCK_CONFIRM_URL`; signed-in buyers alerted
  on their account), one message per restock wave through `notifications`, stop link in every message;
  `Inventory::restock_alerts_of` answers.
