// The copy buttons next to install commands.
document.addEventListener("click", async (e) => {
  const button = e.target.closest("[data-copy]");
  if (!button) return;
  try {
    await navigator.clipboard.writeText(button.dataset.copy);
    button.textContent = "copied";
  } catch (_) {
    // No clipboard (an old browser, or not https): select the command for a manual copy.
    const code = button.parentElement.querySelector("code");
    window.getSelection().selectAllChildren(code);
    button.textContent = "select";
  }
  setTimeout(() => (button.textContent = "copy"), 1500);
});
