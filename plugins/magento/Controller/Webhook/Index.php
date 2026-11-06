<?php
declare(strict_types=1);

namespace Zyvor\Payments\Controller\Webhook;

use Magento\Framework\App\Action\HttpPostActionInterface;
use Magento\Framework\App\CsrfAwareActionInterface;
use Magento\Framework\App\Request\InvalidRequestException;
use Magento\Framework\App\RequestInterface;
use Magento\Framework\Controller\Result\RawFactory;

/**
 * Stub: verify Zyvor HMAC (`Zyvor-Signature`) and mark the Magento order paid on `payment.captured`.
 * Match via external_ref `magento-{increment_id}`. Not wired end-to-end in this scaffold.
 */
class Index implements HttpPostActionInterface, CsrfAwareActionInterface
{
    public function __construct(private readonly RawFactory $rawFactory)
    {
    }

    public function createCsrfValidationException(RequestInterface $request): ?InvalidRequestException
    {
        return null;
    }

    public function validateForCsrf(RequestInterface $request): ?bool
    {
        return true;
    }

    public function execute()
    {
        // TODO: verify signature, load order by external_ref, set state to processing/paid.
        $result = $this->rawFactory->create();
        $result->setHttpResponseCode(204);
        return $result;
    }
}
