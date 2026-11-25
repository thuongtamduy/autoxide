//! One page of a list that is not in the database (step 70 P9, one list shape for every endpoint): built from
//! code (the module registry, a catalog compiled into the program) or fixed reference data. A list from the
//! database never uses this: it pages with a keyset in the database (`fetch_page`, `fetch_page_by`) and has
//! no read cap (D-20261125-list-pages). Items are ordered by a unique text key compared byte by byte, so the
//! order (and the cursor) is the same everywhere.

use uuid::Uuid;

use super::{KeyCursor, Order, Page, PageRequest, Pagination, Position, SortValue, wrong_cursor};
use crate::error::Error;

/// Column name the cursor of [`page_of`] carries.
const KEY: &str = "key";

/// One page of `items`, a list not in the database, ordered by `key` (unique per item; compared byte by byte)
/// in the request's order. Two items with the same key are a programming error (one of them could never be
/// reached): an internal error. The cursor carries the key of the last item, so the next page starts right after
/// it even if items were added or removed in between. `pagination.total` (with `count=true`) is the number of
/// items.
pub fn page_of<T>(
    mut items: Vec<T>,
    key: impl Fn(&T) -> String,
    request: &PageRequest,
) -> Result<Page<T>, Error> {
    let total = request.wants_total().then(|| u64::try_from(items.len()).unwrap_or(u64::MAX));
    // Khoá tính một lần mỗi phần tử; sắp theo byte nên hai database (và mọi máy) ra cùng thứ tự.
    let mut keyed: Vec<(String, T)> = items.drain(..).map(|item| (key(&item), item)).collect();
    keyed.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
    if let Some(pair) = keyed.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(Error::internal(format!(
            "a paged list has two items with the key `{}`: the key must be unique",
            pair[0].0
        )));
    }
    if request.order == Order::Desc {
        keyed.reverse();
    }
    let start = match &request.position {
        None => 0,
        Some(Position::Key(cursor)) if cursor.column == KEY => {
            let SortValue::Text(after) = &cursor.value else { return Err(wrong_cursor()) };
            let after = after.as_bytes();
            keyed
                .iter()
                .position(|(key, _)| match request.order {
                    Order::Asc => key.as_bytes() > after,
                    Order::Desc => key.as_bytes() < after,
                })
                .unwrap_or(keyed.len())
        }
        Some(_) => return Err(wrong_cursor()),
    };
    let limit = request.limit as usize;
    let mut rest = keyed.split_off(start.min(keyed.len()));
    let has_more = rest.len() > limit;
    rest.truncate(limit);
    let next_cursor = has_more.then(|| rest.last()).flatten().map(|(key, _)| {
        KeyCursor { column: KEY.to_owned(), value: SortValue::Text(key.clone()), id: Uuid::nil() }
            .encode()
    });
    Ok(Page {
        data: rest.into_iter().map(|(_, item)| item).collect(),
        pagination: Pagination { limit: request.limit, next_cursor, total },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pagination::{NaturalOrderPageQuery, PageQuery};

    fn request(limit: &str, cursor: Option<String>, order: Option<&str>) -> PageRequest {
        let query =
            NaturalOrderPageQuery::new(Some(limit.to_owned()), cursor, order.map(str::to_owned))
                .with_count(true);
        PageRequest::try_from(query).unwrap()
    }

    fn names(page: &Page<&'static str>) -> Vec<&'static str> {
        page.data.clone()
    }

    #[test]
    fn duyet_het_danh_sach_trong_bo_nho_theo_khoa() {
        let items = vec!["delta", "alpha", "charlie", "bravo", "echo"];
        let first =
            page_of(items.clone(), |item| (*item).to_owned(), &request("2", None, None)).unwrap();
        assert_eq!(names(&first), ["alpha", "bravo"]);
        assert_eq!(first.pagination.total, Some(5));
        let cursor = first.pagination.next_cursor.clone();
        let second =
            page_of(items.clone(), |item| (*item).to_owned(), &request("2", cursor, None)).unwrap();
        assert_eq!(names(&second), ["charlie", "delta"]);
        assert_eq!(second.pagination.total, None, "only the first page counts");
        // Một phần tử thêm vào giữa chừng không làm trùng hay sót phần đã qua.
        let mut grown = items.clone();
        grown.push("aardvark");
        let third = page_of(
            grown,
            |item| (*item).to_owned(),
            &request("2", second.pagination.next_cursor.clone(), None),
        )
        .unwrap();
        assert_eq!(names(&third), ["echo"]);
        assert_eq!(third.pagination.next_cursor, None);

        let reversed = page_of(items, |item| (*item).to_owned(), &request("3", None, Some("desc")));
        assert_eq!(names(&reversed.unwrap()), ["echo", "delta", "charlie"]);
    }

    #[test]
    fn khoa_trung_la_loi_khong_mat_phan_tu_trong_im_lang() {
        let items = vec!["a", "b", "b", "c"];
        let error = page_of(items, |item| (*item).to_owned(), &request("2", None, None));
        assert!(error.is_err());
    }

    #[test]
    fn con_tro_la_bi_tu_choi_va_mac_dinh_tang_dan() {
        let id_cursor = Some(Uuid::now_v7().to_string());
        let error = page_of(vec!["a"], |item| (*item).to_owned(), &request("2", id_cursor, None));
        assert!(error.is_err());
        assert_eq!(request("2", None, None).order(), Order::Asc);
        assert_eq!(PageRequest::try_from(PageQuery::default()).unwrap().order(), Order::Desc);
        let too_many = NaturalOrderPageQuery::new(Some("201".to_owned()), None, None);
        assert!(PageRequest::try_from(too_many).is_err());
    }
}
