//! m0012: lượng tịnh của biến thể (`net_quantity` + `net_unit`) và đơn giá theo đơn vị cơ sở trên
//! storefront (D-201). SQLite chỉ thêm được một cột mỗi lệnh `ALTER TABLE`: thêm từng cột.

use autoxide_kernel::migration::prelude::*;

pub(super) struct AddNetContent;

impl MigrationName for AddNetContent {
    fn name(&self) -> &str {
        "m0012_net_content"
    }
}

const COLUMNS: [(&str, &str); 4] = [
    ("catalog_variants", "net_quantity"),
    ("catalog_variants", "net_unit"),
    ("catalog_storefront_entries", "unit_price_minor"),
    ("catalog_storefront_entries", "unit_price_unit"),
];

#[async_trait::async_trait]
impl MigrationTrait for AddNetContent {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, column) in COLUMNS {
            let mut definition = if column.ends_with("unit") {
                schema::string_len_null(column, 8)
            } else {
                schema::big_integer_null(column)
            };
            manager
                .alter_table(Table::alter().table(table).add_column(&mut definition).to_owned())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, column) in COLUMNS.iter().rev() {
            manager
                .alter_table(Table::alter().table(*table).drop_column(*column).to_owned())
                .await?;
        }
        Ok(())
    }
}
