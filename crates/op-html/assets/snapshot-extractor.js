(function () {
  "use strict";

  var MAX_NODES = 20000;
  var MAX_IMAGE_EDGE = 2048;
  var MAX_IMAGE_DATA_BYTES = 24 * 1024 * 1024;
  var GRAY_PLACEHOLDER =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
  var SKIP_TAGS = {
    SCRIPT: true,
    STYLE: true,
    NOSCRIPT: true,
    TEMPLATE: true,
    META: true,
    LINK: true,
    HEAD: true,
  };
  var ELEMENT_STYLE_KEYS = [
    "background-color",
    "background-image",
    "border-radius",
    "box-shadow",
    "border",
    "opacity",
    "overflow",
    "transform",
    "object-fit",
  ];
  var TEXT_STYLE_KEYS = [
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "text-align",
  ];

  var nodeCount = 0;
  var embeddedImageBytes = 0;
  var remoteImageCount = 0;
  var truncated = false;

  function round(value) {
    return Math.round(value * 100) / 100;
  }

  function pageRect(rect) {
    return {
      x: round(rect.left + window.scrollX),
      y: round(rect.top + window.scrollY),
      w: round(rect.width),
      h: round(rect.height),
    };
  }

  function takeNode() {
    if (nodeCount >= MAX_NODES) {
      truncated = true;
      return false;
    }
    nodeCount += 1;
    return true;
  }

  function copyStyles(computed, keys) {
    var result = {};
    keys.forEach(function (key) {
      result[key] = computed.getPropertyValue(key);
    });
    return result;
  }

  function childHasVisibleBox(element) {
    return Array.prototype.some.call(element.children, function (child) {
      var style = window.getComputedStyle(child);
      var rect = child.getBoundingClientRect();
      return (
        style.display !== "none" &&
        style.visibility !== "hidden" &&
        Number(style.opacity) !== 0 &&
        (rect.width >= 0.5 || rect.height >= 0.5)
      );
    });
  }

  function visibleElement(element, computed, rect) {
    if (
      computed.display === "none" ||
      computed.visibility === "hidden" ||
      Number(computed.opacity) === 0
    ) {
      return false;
    }
    if (rect.width < 0.5 || rect.height < 0.5) {
      return childHasVisibleBox(element);
    }
    return true;
  }

  function buildText(textNode, parentComputed) {
    var text = (textNode.textContent || "").replace(/\s+/g, " ").trim();
    if (!text) return null;
    var range = document.createRange();
    range.selectNodeContents(textNode);
    var rect = range.getBoundingClientRect();
    if (typeof range.detach === "function") range.detach();
    if (rect.width < 0.5 || rect.height < 0.5 || !takeNode()) return null;
    return {
      kind: "text",
      rect: pageRect(rect),
      text: text,
      styles: copyStyles(parentComputed, TEXT_STYLE_KEYS),
    };
  }

  function scaledCanvas(width, height) {
    var longest = Math.max(width, height, 1);
    var scale = Math.min(1, MAX_IMAGE_EDGE / longest);
    var canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    return canvas;
  }

  function keepDataUrl(dataUrl, fallbackUrl) {
    if (!dataUrl || embeddedImageBytes + dataUrl.length > MAX_IMAGE_DATA_BYTES) {
      remoteImageCount += 1;
      return { src: fallbackUrl || GRAY_PLACEHOLDER, tainted: true };
    }
    embeddedImageBytes += dataUrl.length;
    return { src: dataUrl, tainted: false };
  }

  function rasterizeImage(image) {
    var fallback = image.currentSrc || image.src || GRAY_PLACEHOLDER;
    if (!image.complete || !image.naturalWidth || !image.naturalHeight) {
      remoteImageCount += 1;
      return { src: fallback, tainted: true };
    }
    try {
      var canvas = scaledCanvas(image.naturalWidth, image.naturalHeight);
      canvas
        .getContext("2d")
        .drawImage(image, 0, 0, canvas.width, canvas.height);
      return keepDataUrl(canvas.toDataURL("image/png"), fallback);
    } catch (_error) {
      remoteImageCount += 1;
      return { src: fallback, tainted: true };
    }
  }

  function encodeSvg(svg) {
    try {
      var xml = new XMLSerializer().serializeToString(svg);
      var encoded = btoa(unescape(encodeURIComponent(xml)));
      return keepDataUrl("data:image/svg+xml;base64," + encoded, GRAY_PLACEHOLDER);
    } catch (_error) {
      remoteImageCount += 1;
      return { src: GRAY_PLACEHOLDER, tainted: true };
    }
  }

  function captureCanvas(canvas) {
    try {
      return keepDataUrl(canvas.toDataURL("image/png"), GRAY_PLACEHOLDER);
    } catch (_error) {
      remoteImageCount += 1;
      return { src: GRAY_PLACEHOLDER, tainted: true };
    }
  }

  function captureVideo(video) {
    try {
      var width = video.videoWidth || video.clientWidth;
      var height = video.videoHeight || video.clientHeight;
      var canvas = scaledCanvas(width, height);
      canvas
        .getContext("2d")
        .drawImage(video, 0, 0, canvas.width, canvas.height);
      return keepDataUrl(canvas.toDataURL("image/png"), GRAY_PLACEHOLDER);
    } catch (_error) {
      remoteImageCount += 1;
      return { src: GRAY_PLACEHOLDER, tainted: true };
    }
  }

  function buildImage(element, computed, rect) {
    if (!takeNode()) return null;
    var captured;
    if (element.tagName === "IMG") captured = rasterizeImage(element);
    else if (element.tagName === "SVG") captured = encodeSvg(element);
    else if (element.tagName === "CANVAS") captured = captureCanvas(element);
    else captured = captureVideo(element);
    var result = {
      kind: "image",
      tag: element.tagName.toLowerCase(),
      rect: pageRect(rect),
      src: captured.src,
      styles: copyStyles(computed, ELEMENT_STYLE_KEYS),
    };
    if (captured.tainted) result.tainted = true;
    return result;
  }

  function buildElement(element) {
    if (SKIP_TAGS[element.tagName]) return null;
    var computed = window.getComputedStyle(element);
    var rect = element.getBoundingClientRect();
    if (!visibleElement(element, computed, rect)) return null;
    if (
      element.tagName === "IMG" ||
      element.tagName === "SVG" ||
      element.tagName === "CANVAS" ||
      element.tagName === "VIDEO"
    ) {
      return buildImage(element, computed, rect);
    }
    if (!takeNode()) return null;
    var children = [];
    Array.prototype.forEach.call(element.childNodes, function (child) {
      if (truncated) return;
      var mapped = null;
      if (child.nodeType === Node.TEXT_NODE) {
        mapped = buildText(child, computed);
      } else if (child.nodeType === Node.ELEMENT_NODE) {
        mapped = buildElement(child);
      }
      if (mapped) children.push(mapped);
    });
    return {
      kind: "element",
      tag: element.tagName.toLowerCase(),
      rect: pageRect(rect),
      styles: copyStyles(computed, ELEMENT_STYLE_KEYS),
      children: children,
    };
  }

  if (!document.body) {
    console.error("OpenPencil snapshot: document.body is not available");
    return;
  }
  var root = buildElement(document.body);
  if (!root) {
    console.error("OpenPencil snapshot: the page has no visible body");
    return;
  }
  var snapshot = {
    version: 1,
    source: window.location.href,
    title: document.title,
    viewport: {
      width: round(window.innerWidth),
      height: round(window.innerHeight),
    },
    root: root,
  };
  if (truncated) snapshot.truncated = true;
  var output = JSON.stringify(snapshot, null, 2);

  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(output).catch(function (error) {
      console.warn("OpenPencil snapshot: clipboard copy failed", error);
    });
  }
  var blobUrl = URL.createObjectURL(
    new Blob([output], { type: "application/json" }),
  );
  var download = document.createElement("a");
  download.href = blobUrl;
  download.download = "snapshot.json";
  download.click();
  setTimeout(function () {
    URL.revokeObjectURL(blobUrl);
  }, 0);
  console.log("OpenPencil snapshot ready", {
    nodes: nodeCount,
    bytes: output.length,
    embeddedImageBytes: embeddedImageBytes,
    remoteImages: remoteImageCount,
    truncated: truncated,
  });
})();
