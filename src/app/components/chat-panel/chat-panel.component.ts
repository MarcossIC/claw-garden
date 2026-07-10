import {
  Component,
  ChangeDetectionStrategy,
  inject,
  signal,
  viewChild,
  ElementRef,
  effect,
} from '@angular/core';
import { ChatService } from '../../services/chat.service';
import { AgentService } from '../../services/agent.service';
import { MarkdownPipe } from '../../pipes/markdown.pipe';

@Component({
  selector: 'app-chat-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkdownPipe],
  templateUrl: './chat-panel.component.html',
  styleUrl: './chat-panel.component.css',
})
export class ChatPanelComponent {
  readonly chatService = inject(ChatService);
  private agentService = inject(AgentService);

  readonly messageList = viewChild<ElementRef<HTMLDivElement>>('messageList');
  readonly inputText = signal('');

  constructor() {
    // Auto-scroll when messages change
    effect(() => {
      this.chatService.messages();
      const el = this.messageList()?.nativeElement;
      if (el) {
        requestAnimationFrame(() => {
          el.scrollTop = el.scrollHeight;
        });
      }
    });

    // Sync active agent with selected agent
    effect(() => {
      const agent = this.agentService.selectedAgent();
      if (agent) {
        this.chatService.activeAgentId.set(agent.id);
      }
    });
  }

  send(): void {
    const agentId = this.chatService.activeAgentId();
    const text = this.inputText();
    if (!agentId || !text.trim()) return;

    this.chatService.sendMessage(agentId, text);
    this.inputText.set('');
  }

  retry(errorMsgId: string): void {
    this.chatService.retryMessage(errorMsgId);
  }

  editAndRetry(errorMsgId: string): void {
    const text = this.chatService.editFailedMessage(errorMsgId);
    if (text) {
      this.inputText.set(text);
    }
  }

  onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      this.send();
    }
  }

  onInput(event: Event): void {
    const target = event.target as HTMLTextAreaElement;
    this.inputText.set(target.value);
  }
}
