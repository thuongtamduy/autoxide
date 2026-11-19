//! Chính sách tách / gộp khách của các bảng của `shipping` (bước 38, D-20261118-step38-relocation):
//! vùng, phương thức, lịch giao, tài khoản hãng và vận đơn theo nút; bảng con đi theo dòng cha; sổ
//! địa chỉ người mua và chênh lệch COD là dữ liệu cả khách, ở lại khách nguồn khi tách.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&[
            "shipping_zones",
            "shipping_methods",
            "shipping_calendars",
            "shipping_carrier_accounts",
            "shipping_shipments",
        ])
        .follow("shipping_zone_regions", "zone_id", "shipping_zones")
        .follow("shipping_rates", "method_id", "shipping_methods")
        .follow("shipping_pickup_points", "method_id", "shipping_methods")
        .follow("shipping_calendar_days", "calendar_id", "shipping_calendars")
        .follow("shipping_parcels", "shipment_id", "shipping_shipments")
        .follow("shipping_shipment_items", "shipment_id", "shipping_shipments")
        .follow("shipping_tracking_events", "shipment_id", "shipping_shipments")
        .follow("shipping_webhook_inbox", "carrier_account_id", "shipping_carrier_accounts")
        .follow("shipping_cod_statements", "carrier_account_id", "shipping_carrier_accounts")
        .follow("shipping_cod_statement_lines", "statement_id", "shipping_cod_statements")
        .stay(&["shipping_addresses", "shipping_cod_discrepancies"])
}
