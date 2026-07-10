import { Pipe, PipeTransform } from '@angular/core';
import { marked } from 'marked';
import DOMPurify from 'dompurify';

// Configure marked for streaming-friendly output
marked.setOptions({
  breaks: true,
  gfm: true,
});

@Pipe({ name: 'markdown', pure: true })
export class MarkdownPipe implements PipeTransform {
  transform(value: string): string {
    if (!value) return '';
    const raw = marked.parse(value, { async: false }) as string;
    return DOMPurify.sanitize(raw);
  }
}
