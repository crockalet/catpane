package dev.catpane.helper

import android.app.Activity
import android.content.Intent
import android.net.VpnService
import android.os.Bundle
import android.view.View
import android.widget.Button
import android.widget.RadioGroup
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity

/**
 * Minimal helper UI: lets the user grant/revoke `BIND_VPN_SERVICE` and choose
 * a default LAN-exclusion mode. Day-to-day operation is driven by CatPane
 * over the [ControlServer] — this activity exists only for one-time setup.
 */
class MainActivity : AppCompatActivity() {

    private lateinit var statusLabel: TextView
    private lateinit var statusDetail: TextView
    private lateinit var grantButton: Button
    private lateinit var stopButton: Button
    private lateinit var lanGroup: RadioGroup

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        statusLabel = findViewById(R.id.status_label)
        statusDetail = findViewById(R.id.status_detail)
        grantButton = findViewById(R.id.grant_button)
        stopButton = findViewById(R.id.stop_button)
        lanGroup = findViewById(R.id.lan_group)

        grantButton.setOnClickListener { requestVpnPermission() }
        stopButton.setOnClickListener { stopVpn() }

        // We start the control server eagerly so CatPane can talk to us as
        // soon as the package is installed (status will report
        // `vpn_permission_granted=false` until the user taps Grant).
        ControlServer.ensureStarted(this)
    }

    override fun onResume() {
        super.onResume()
        refreshStatus()
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_VPN_PERMISSION && resultCode == Activity.RESULT_OK) {
            // The system has granted permission; the next `apply` from CatPane
            // will start the VPN. We don't auto-start with empty config.
            statusDetail.text = "Permission granted. CatPane can now apply network conditions."
        }
        refreshStatus()
    }

    private fun requestVpnPermission() {
        val intent = VpnService.prepare(this)
        if (intent != null) {
            startActivityForResult(intent, REQ_VPN_PERMISSION)
        } else {
            statusDetail.text = "Permission already granted."
            refreshStatus()
        }
    }

    private fun stopVpn() {
        ThrottleVpnService.instance?.stopShaping()
        stopService(Intent(this, ThrottleVpnService::class.java))
        refreshStatus()
    }

    private fun refreshStatus() {
        val granted = VpnService.prepare(this) == null
        val running = ThrottleVpnService.instance?.let {
            it.snapshotStatus(null).running
        } ?: false
        statusLabel.text = if (running) getString(R.string.status_active)
        else getString(R.string.status_idle)
        grantButton.visibility = if (granted) View.GONE else View.VISIBLE
        stopButton.isEnabled = running
    }

    companion object {
        private const val REQ_VPN_PERMISSION = 9001
    }
}
