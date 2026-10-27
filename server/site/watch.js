// A score's page opened from a "Watch replay" button: straight on into the game.
const open = document.querySelector("[data-autoopen]");
if (open) location.href = open.href;
