// jsdom lacks layout APIs the panes call.
Element.prototype.scrollIntoView ??= function () {};
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
