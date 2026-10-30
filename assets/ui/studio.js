(function() {
  "use strict";
  var themeToggle = document.getElementById("theme-toggle");
  var savedTheme = localStorage.getItem("gw_theme");
  if (savedTheme === "dark") {
    document.body.classList.add("dark");
  }
  if (themeToggle) {
    themeToggle.addEventListener("click", function() {
      var isDark = document.body.classList.toggle("dark");
      localStorage.setItem("gw_theme", isDark ? "dark" : "light");
    });
  }
})();
