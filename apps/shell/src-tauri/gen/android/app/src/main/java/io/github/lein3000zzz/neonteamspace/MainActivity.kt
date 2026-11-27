package io.github.lein3000zzz.neonteamspace

import android.app.DownloadManager
import android.content.res.Configuration
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.view.View
import android.webkit.CookieManager
import android.webkit.URLUtil
import android.webkit.WebView
import android.widget.Toast
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

// The page the app loads on its way back to the picker, in place of a server's.
private const val BLANK = "about:blank"

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // The app draws behind the system bars, which the WebView does not tell pages about as
    // safe-area insets: the webview sits between the bars, and above the keyboard, instead.
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout() or
          WindowInsetsCompat.Type.ime()
      )
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }

  // The manifest keeps a switch between light and dark mode from recreating the activity,
  // which would load the webview's page again and lose what is on it. The page follows the
  // mode by itself; the window behind the system bars, and the bars' icons, follow it here.
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    window.setBackgroundDrawableResource(R.color.window_background)
    enableEdgeToEdge()
  }

  override fun onWebViewCreate(webView: WebView) {
    goBackThroughHistory(webView)
    saveDownloads(webView)
  }

  // Back goes back in the webview's history while there is any: through the server's pages
  // and the entries its open panels and menus add, which going back closes, and from the
  // server's first page to the picker. With none left, it leaves the app.
  private fun goBackThroughHistory(webView: WebView) {
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        if (hasPageBehind(webView)) {
          webView.goBack()
        } else {
          // What is left behind is of no use. Without it, Tauri's own handler, which comes
          // next and goes back wherever the webview can, leaves the app too.
          webView.clearHistory()
          isEnabled = false
          onBackPressedDispatcher.onBackPressed()
          isEnabled = true
        }
      }
    })
  }

  // Whether going back leads to a page. When the app shows the picker again itself, after
  // Stop, after a server's page that did not come in time, or when a server's page asks for
  // the app's own address, it loads a blank page and then the picker: from that picker, as
  // from the first one, there is nothing to go back to. Android does not ask the app about a
  // step back in the history, so the blank page cannot be refused there as it is elsewhere.
  private fun hasPageBehind(webView: WebView): Boolean {
    val history = webView.copyBackForwardList()
    val at = history.currentIndex
    if (at < 1) {
      return false
    }
    val behind = history.getItemAtIndex(at - 1)?.url
    val here = history.currentItem?.url?.let(Uri::parse)
    // The first page the window loaded is the picker, on the app's own origin.
    val picker = history.getItemAtIndex(0)?.url?.let(Uri::parse)
    val onPicker = here != null && picker != null &&
      here.scheme == picker.scheme &&
      here.host == picker.host &&
      here.port == picker.port
    return !(onPicker && behind == BLANK)
  }

  // Android's WebView saves no file by itself. The system's download manager fetches the
  // server's, with the page's cookies, from the origin of the page alone, which the app's
  // origin lock keeps on the server; anything else, a file the page made itself among them, is
  // refused with a note.
  private fun saveDownloads(webView: WebView) {
    webView.setDownloadListener { url, userAgent, contentDisposition, mimeType, _ ->
      val file = Uri.parse(url)
      val page = webView.url?.let(Uri::parse)
      val sameOrigin = page != null &&
        file.scheme in listOf("https", "http") &&
        file.scheme == page.scheme &&
        file.host == page.host &&
        file.port == page.port
      if (!sameOrigin) {
        Toast.makeText(this, getString(R.string.download_refused), Toast.LENGTH_LONG).show()
        return@setDownloadListener
      }
      val name = fileName(url, contentDisposition, mimeType)
      val request = DownloadManager.Request(file)
        .setTitle(name)
        .setMimeType(mimeType)
        .addRequestHeader("User-Agent", userAgent)
        .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
      CookieManager.getInstance().getCookie(url)?.let { request.addRequestHeader("Cookie", it) }
      // The shared Downloads folder needs no permission from Android 10 on; before it, the
      // app's own, which the download's notification opens.
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        request.setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, name)
      } else {
        request.setDestinationInExternalFilesDir(this, Environment.DIRECTORY_DOWNLOADS, name)
      }
      getSystemService(DownloadManager::class.java).enqueue(request)
      Toast.makeText(this, getString(R.string.download_started, name), Toast.LENGTH_SHORT).show()
    }
  }

  // The name the server gives the file: Content-Disposition's filename*, which carries any
  // name in UTF-8, then its filename, which URLUtil only reads when it comes alone.
  private fun fileName(url: String, disposition: String?, mimeType: String?): String {
    val named = disposition?.let {
      Regex("filename\\*\\s*=\\s*UTF-8''([^;\\s]+)", RegexOption.IGNORE_CASE).find(it)
        ?.let { match -> Uri.decode(match.groupValues[1]) }
        ?: Regex("filename\\s*=\\s*\"([^\"]+)\"", RegexOption.IGNORE_CASE).find(it)
          ?.groupValues?.get(1)
    }
    val name = named ?: URLUtil.guessFileName(url, disposition, mimeType)
    return name.replace('/', '_').replace('\\', '_')
  }
}
