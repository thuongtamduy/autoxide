// Hành vi chung của khung quản trị, dùng cho mọi màn:
// - Ctrl K / ⌘K mở ô tìm, "?" mở trợ giúp (Esc đóng: popover tự làm);
// - lọc danh sách của ô tìm, không phân biệt dấu; Enter mở mục đầu tiên;
// - cuộn mục đang mở của dải tab vào giữa, để trên điện thoại luôn thấy mình đang ở đâu.
(function () {
  const norm = (s) => s.toLowerCase().normalize('NFD').replace(/đ/g, 'd').replace(/[̀-ͯ]/g, '').trim();
  const typing = (el) => el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));

  document.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'k') {
      const opener = document.querySelector('[data-cmdk]');
      if (opener) { e.preventDefault(); opener.click(); }
      return;
    }
    if (e.key === '?' && !typing(e.target)) {
      const help = document.getElementById('help');
      if (help && help.showPopover) { e.preventDefault(); try { help.showPopover(); } catch (_) { /* đã mở */ } }
    }
  });

  const filter = (input) => {
    const q = norm(input.value);
    const box = input.closest('.axsheet');
    let shown = 0;
    box.querySelectorAll('.qs-it').forEach((a) => {
      const hit = !q || a.dataset.k.includes(q);
      a.hidden = !hit;
      if (hit) shown++;
    });
    const none = box.querySelector('.qs-none');
    if (none) none.hidden = shown > 0;
  };
  document.addEventListener('input', (e) => { if (e.target.closest && e.target.closest('#qs')) filter(e.target); });
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Enter' || !e.target.closest || !e.target.closest('#qs .qs-in')) return;
    const first = [...document.querySelectorAll('#qs .qs-it')].find((a) => !a.hidden);
    if (first) { e.preventDefault(); first.click(); }
  });
  document.addEventListener('toggle', (e) => {
    if (e.target.id !== 'qs' || e.newState !== 'open') return;
    const input = e.target.querySelector('input');
    if (input) { input.value = ''; filter(input); input.focus(); }
  }, true);

  // Cuộn mục đang mở vào giữa dải, chỉ khi mục đang mở đổi (không giành cuộn với người dùng)
  const seen = new WeakMap();
  const center = () => {
    document.querySelectorAll('nav.subnav, .ptabs, .scrollrow, .chiprow').forEach((nav) => {
      if (nav.scrollWidth <= nav.clientWidth + 1) return;
      const on = nav.querySelector('.on, [aria-current="page"], [aria-selected="true"]');
      if (!on || seen.get(nav) === on) return;
      seen.set(nav, on);
      const a = on.getBoundingClientRect(), b = nav.getBoundingClientRect();
      const left = nav.scrollLeft + (a.left - b.left) - (nav.clientWidth - a.width) / 2;
      nav.scrollLeft = Math.max(0, left);
    });
  };
  let timer = 0;
  const later = () => { clearTimeout(timer); timer = setTimeout(center, 60); };
  const start = () => { later(); new MutationObserver(later).observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['class', 'aria-current', 'aria-selected'] }); };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start); else start();
})();
