import "./picker.css";
import { Picker } from "./picker";
import { followTheSystem, setUpPaletteChoice } from "./theme";
import { Updates } from "./updates";

followTheSystem();
setUpPaletteChoice(document.querySelector("fieldset"));

function element(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (found === null) {
    throw new Error(`index.html has no #${id}`);
  }
  return found;
}

// Coming back from a server with the back gesture can restore the picker as it was left,
// busy opening that server; it loads afresh instead, and shows the list.
window.addEventListener("pageshow", (event) => {
  if (event.persisted) {
    window.location.reload();
  }
});

const updates = new Updates(element("updates"));

void new Picker(
  element("view"),
  element("title"),
  element("status"),
  element("picker"),
  element("actions"),
  (startup) => {
    if (startup.updateCheck) {
      updates.show();
    }
  },
).start();
