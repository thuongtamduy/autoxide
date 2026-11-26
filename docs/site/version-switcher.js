// Bộ chọn phiên bản cho website tài liệu (D-20261126-docs-site-mdbook): đọc versions.json ở gốc site và thêm một ô chọn
// vào thanh trên cùng của mdBook. Đổi phiên bản hay ngôn ngữ giữ nguyên trang đang xem khi trang đó có ở bản kia.
(function () {
  "use strict";
  // Đường dẫn dạng /<gốc site>/<phiên bản>/<ngôn ngữ>/<trang>; gốc site có thể rỗng hay là tên repo (GitHub Pages).
  var path = window.location.pathname;
  var match = path.match(/^(.*?)\/(v\d[^/]*|latest|main)\/(en|vi)\/(.*)$/);
  if (!match) return;
  var base = match[1], version = match[2], lang = match[3], page = match[4];
  fetch(base + "/versions.json")
    .then(function (r) { return r.json(); })
    .then(function (data) {
      var bar = document.querySelector(".right-buttons");
      if (!bar) return;
      var make = function (options, current, build) {
        var select = document.createElement("select");
        select.setAttribute("aria-label", "version or language");
        options.forEach(function (o) {
          var el = document.createElement("option");
          el.value = o; el.textContent = o; el.selected = o === current;
          select.appendChild(el);
        });
        select.addEventListener("change", function () { window.location.href = build(select.value); });
        return select;
      };
      var versions = ["latest"].concat(data.versions || []);
      bar.prepend(make(["en", "vi"], lang, function (l) { return base + "/" + version + "/" + l + "/" + page; }));
      bar.prepend(make(versions, version, function (v) { return base + "/" + v + "/" + lang + "/" + page; }));
    })
    .catch(function () { /* không có versions.json (bản dựng cục bộ): bỏ qua */ });
})();
