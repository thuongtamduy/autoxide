- Module `orders`: buyer side of returns — `POST /me/orders/{id}/returns/preview` shows the refund (with discounts taken
  back), the ways to send items back (`pickup`, `drop_off`, `in_store`) and their fee, when the refund should arrive and
  the deadline, without saving anything; photos must be uploaded by the buyer and are required for the reasons in the
  new setting `orders.return_photo_reasons`; approving a `pickup` return asks the shipping module for a carrier pickup
  (`orders.return_pickup_requested`).
