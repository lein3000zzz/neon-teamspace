import "./picker.css";
import { Picker } from "./picker";
import { followTheSystem, setUpPaletteChoice } from "./theme";

followTheSystem();
setUpPaletteChoice(document.querySelector("fieldset"));

function element(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (found === null) {
    throw new Error(`index.html has no #${id}`);
  }
  return found;
}

void new Picker(element("view"), element("title"), element("status"), element("picker")).start();
