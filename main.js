document.querySelectorAll(".menu").forEach((button) => {
  button.addEventListener("click", () => {
    const nav = document.getElementById("links");
    const open = nav.classList.toggle("open");
    button.setAttribute("aria-expanded", open ? "true" : "false");
  });
});
