import React from 'react';
import { Button, Card } from '@panda/ui/primitives';
import { reportClientError } from '@panda/ui/lib/clientDiagnostics';

interface AppErrorBoundaryProps {
  children: React.ReactNode;
}

interface AppErrorBoundaryState {
  failure: React.ReactNode;
}

export class AppErrorBoundary extends React.Component<AppErrorBoundaryProps, AppErrorBoundaryState> {
  override state: AppErrorBoundaryState = { failure: null };

  static getDerivedStateFromError(): AppErrorBoundaryState {
    return { failure: <AppFailure /> };
  }

  override componentDidCatch(error: Error, info: React.ErrorInfo) {
    const details = [error.message];
    if (info.componentStack) details.push(info.componentStack);
    const diagnosticError = details.length > 1 ? new Error(details.join('\n')) : error;
    reportClientError(diagnosticError, {
      event: 'react.render_failed',
      area: 'render',
      operation: 'app-root',
      severity: 'error',
    });
  }

  override render() {
    return this.state.failure ?? this.props.children;
  }
}

function AppFailure() {
  return (
      <main className="flex min-h-screen items-center justify-center bg-bg-page p-6" role="alert">
        <Card className="w-full max-w-md text-center">
          <h1 className="text-lg font-semibold text-fg">Panda needs to reload</h1>
          <p className="mt-2 text-sm text-fg-secondary">
            The app hit an unexpected error. Reload to try again.
          </p>
          <Button type="button" className="mt-5" onClick={() => window.location.reload()}>
            Reload app
          </Button>
        </Card>
      </main>
  );
}
