<script>
// Sidebar section fixes. Quarto renders text-only sections with an empty id
// on the ul and data-bs-target="#" on the toggles, which Bootstrap resolves
// to nothing, leaving the toggle inert. Give each such ul a real id and
// point its toggles at it. Then collapse the "Developer details" section by
// default, unless the current page lives inside it — Quarto only offers a
// sidebar-wide collapse-level, so this targets the one section.
document.addEventListener("DOMContentLoaded", function () {
  document.querySelectorAll("li.sidebar-item-section").forEach(function (li, i) {
    var ul = li.querySelector("ul.collapse");
    if (ul && !ul.id) {
      ul.id = "sidebar-section-" + i;
      li.querySelectorAll('[data-bs-toggle="collapse"]').forEach(function (a) {
        a.setAttribute("data-bs-target", "#" + ul.id);
      });
    }
  });

  document.querySelectorAll("li.sidebar-item-section").forEach(function (li) {
    var label = li.querySelector(".sidebar-item-container .menu-text");
    if (!label || label.textContent.trim() !== "Developer details") return;
    if (li.querySelector(".sidebar-link.active")) return;
    var ul = li.querySelector("ul.collapse");
    if (ul) ul.classList.remove("show");
    li.querySelectorAll('[aria-expanded="true"]').forEach(function (a) {
      a.setAttribute("aria-expanded", "false");
    });
  });
});
</script>
