import { useEffect, useRef, useState } from "react";
import * as pdfjs from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

/** Draws every page of a PDF onto canvases, so the preview does not depend on the browser's PDF viewer. */
export function PdfView({ data }: { data: ArrayBuffer }) {
  const host = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    let cancelled = false;
    element.replaceChildren();
    setError(null);
    const task = pdfjs.getDocument({ data: new Uint8Array(data) });
    task.promise
      .then(async (pdf) => {
        const width = Math.max(element.clientWidth - 2, 200);
        for (let number = 1; number <= pdf.numPages; number++) {
          if (cancelled) return;
          const page = await pdf.getPage(number);
          const base = page.getViewport({ scale: 1 });
          const scale = width / base.width;
          const viewport = page.getViewport({ scale: scale * window.devicePixelRatio });
          const canvas = document.createElement("canvas");
          canvas.className = "pdf-page";
          canvas.width = Math.floor(viewport.width);
          canvas.height = Math.floor(viewport.height);
          canvas.style.width = `${width}px`;
          canvas.style.height = `${Math.floor(base.height * scale)}px`;
          element.appendChild(canvas);
          const context = canvas.getContext("2d");
          if (!context) throw new Error("This browser cannot draw the preview.");
          await page.render({ canvasContext: context, viewport }).promise;
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : "Could not show the PDF.");
      });
    return () => {
      cancelled = true;
      void task.destroy();
    };
  }, [data]);

  return (
    <div className="pdf-view">
      {error && <p className="banner error">{error}</p>}
      <div ref={host} />
    </div>
  );
}
