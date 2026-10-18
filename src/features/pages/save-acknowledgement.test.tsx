// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, expect, it } from "vite-plus/test";
import { useSaveAcknowledgement } from "./save-acknowledgement";

beforeAll(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
});
const cleanups: Array<() => void> = [];
afterEach(() => {
  cleanups.splice(0).forEach((cleanup) => cleanup());
});

function mount() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let acknowledge!: (baseRevision: string, snapshot: { revision: string; text: string }) => void;
  function View({
    identity,
    text,
    revision,
  }: {
    identity: string;
    text: string;
    revision: string;
  }) {
    const [persisted, ack] = useSaveAcknowledgement(identity, { text, revision });
    acknowledge = ack;
    return <span>{persisted.text}</span>;
  }
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return {
    render(text: string, revision: string, identity = "block") {
      act(() => root.render(<View identity={identity} text={text} revision={revision} />));
    },
    acknowledge(baseRevision: string, text: string, revision: string) {
      act(() => acknowledge(baseRevision, { text, revision }));
    },
    text: () => container.textContent,
  };
}

it("bridges only the superseded base and retires after the matching Query snapshot arrives", () => {
  const view = mount();
  view.render("Base", "r1");
  view.acknowledge("r1", "Saved", "r2");
  expect(view.text()).toBe("Saved");
  view.render("Saved", "r2");
  view.render("Base", "r1");
  expect(view.text()).toBe("Base");
});

it("lets an advanced remote snapshot win over a late acknowledgement without resurrecting it", () => {
  const view = mount();
  view.render("Base", "r1");
  view.render("Remote", "r3");
  view.acknowledge("r1", "Older saved", "r2");
  expect(view.text()).toBe("Remote");
  view.render("Base", "r1");
  expect(view.text()).toBe("Base");
});

it("keeps consecutive local acknowledgements over their known unobserved baselines", () => {
  const view = mount();
  view.render("Base", "r1");
  view.acknowledge("r1", "First saved", "r2");
  view.acknowledge("r2", "Latest saved", "r3");
  expect(view.text()).toBe("Latest saved");
  view.render("First saved", "r2");
  expect(view.text()).toBe("Latest saved");
  view.render("Latest saved", "r3");
  view.render("Base", "r1");
  expect(view.text()).toBe("Base");
});

it("does not carry an acknowledgement across editor identities or navigation back", () => {
  const view = mount();
  view.render("First", "r1", "first");
  view.acknowledge("r1", "First saved", "r2");
  view.render("Other", "r1", "other");
  expect(view.text()).toBe("Other");
  view.render("First", "r1", "first");
  expect(view.text()).toBe("First");
});
