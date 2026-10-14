// Ambient types for the app.
declare global {
  namespace App {
    // Shallow-routing state: lets the hardware Back button close an open sheet.
    interface PageState {
      sheet?: boolean;
    }
  }
}

export {};
