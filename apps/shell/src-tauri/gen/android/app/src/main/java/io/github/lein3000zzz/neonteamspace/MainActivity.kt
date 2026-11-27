package io.github.lein3000zzz.neonteamspace

import android.app.DownloadManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.webkit.CookieManager
import android.webkit.URLUtil
import android.webkit.WebView
import android.widget.Toast
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // Android's WebView saves no file by itself. The system's download manager fetches the
  // server's, with the page's cookies, from the origin of the page alone, which the app's
  // origin lock keeps on the server; anything else, a file the page made itself among them, is
  // refused with a note.
  override fun onWebViewCreate(webView: WebView) {
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
      val name = URLUtil.guessFileName(url, contentDisposition, mimeType)
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
}
