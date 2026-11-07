package dev.zyvor.vayora.kit

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.URI

class ConnectionSetupTest {
    @Test
    fun parseAndThisPhone() {
        assertTrue(ConnectionCheck.parse("http://10.0.2.2:8080") != null)
        assertTrue(ConnectionCheck.isThisPhone(URI("http://127.0.0.1:8080")))
        assertFalse(ConnectionCheck.isThisPhone(URI("http://10.0.2.2:8080")))
    }

    @Test
    fun classifyOkAndWrongKey() {
        val addr = URI("http://10.0.2.2:8080")
        assertEquals(
            ConnectionVerdict.Ok,
            ConnectionCheck.classify(addr, Probe.Status(200), Probe.Status(200)),
        )
        assertEquals(
            ConnectionVerdict.WrongKey,
            ConnectionCheck.classify(addr, Probe.Status(200), Probe.Status(401)),
        )
        assertEquals(
            ConnectionVerdict.NoKey,
            ConnectionCheck.classify(addr, Probe.Status(200), null, needsKey = true),
        )
    }

    @Test
    fun setupPayloadAcceptsTestKeyOnly() {
        val p = SetupPayload.parse("vayora://setup?server=http%3A%2F%2F10.0.2.2%3A8080&key=zk_test_abc")
        assertEquals("http://10.0.2.2:8080", p.server)
        assertEquals("zk_test_abc", p.apiKey)
        try {
            SetupPayload.parse("vayora://setup?server=http://10.0.2.2:8080&key=zk_live_no")
            throw AssertionError("expected live key refused")
        } catch (_: SetupPayload.Failure.LiveKeyRefused) {
        }
    }
}
